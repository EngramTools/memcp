//! Shared per-message ingest pipeline helper (Phase 24.5 Plan 03).
//!
//! Hosts `process_ingest_message`, the canonical per-message body used by both:
//!   - `pipeline/auto_store/mod.rs` — file-watching JSONL worker
//!   - `transport/api/ingest.rs`    — POST /v1/ingest HTTP handler
//!
//! Pipeline order per D-10 (ingest parity with auto-store):
//!   1. Redaction (fail-closed)
//!   2. ContentFilter.check (Drop => Filtered outcome)
//!   3. Optional category classifier (for salience seed 2.5 vs 1.5)
//!   4. Summarize if role == "assistant" and provider configured (else passthrough)
//!   5. Build tags: session:<id>, project:<name>, category:<label>, summarized
//!   6. Extract event_time
//!   7. Build CreateMemory + `store.store_with_outcome`
//!   8. On Created: seed salience + enqueue embedding + enqueue extraction
//!
//! NOT in the helper (stays in worker): companion `.ids.jsonl` emission,
//! chunking (excluded from ingest per D-10), `daemon_status` update.

use std::sync::Arc;

use crate::content_filter::{ContentFilter, FilterVerdict};
use crate::embedding::{build_embedding_text, EmbeddingJob};
use crate::extraction::ExtractionJob;
use crate::pipeline::auto_store::filter::CategoryResult;
use crate::pipeline::redaction::RedactionEngine;
use crate::pipeline::temporal::extract_event_time;
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{CreateMemory, MemoryStore, StoreOutcome};
use crate::summarization::SummarizationProvider;

pub type EmbedSender = tokio::sync::mpsc::Sender<EmbeddingJob>;
pub type ExtractSender = tokio::sync::mpsc::Sender<ExtractionJob>;

/// Input tuple for a single-message pipeline run.
pub struct ProcessMessageInput<'a> {
    pub source: &'a str,
    pub session_id: &'a str,
    pub project: &'a str,
    pub role: &'a str,
    pub content: &'a str,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub idempotency_key: Option<String>,
    pub reply_to_id: Option<String>,
    pub actor: Option<String>,
    /// `auto_store` | `ingest` | etc. Influences tier inference and logs.
    pub write_path: &'static str,
    /// Baseline tag set (e.g. `["auto-store"]` or empty for ingest).
    pub base_tags: Vec<String>,
    /// Optional pre-computed category classification (auto-store path uses this).
    pub category: Option<CategoryResult>,
    pub birth_year: Option<u32>,
}

/// Dependencies wired at boot. Cheap to build per call.
pub struct ProcessMessageContext<'a> {
    pub store: &'a Arc<PostgresMemoryStore>,
    pub redaction_engine: Option<&'a RedactionEngine>,
    pub content_filter: Option<&'a Arc<dyn ContentFilter>>,
    pub summarization_provider: Option<&'a Arc<dyn SummarizationProvider>>,
    pub embed_sender: Option<&'a EmbedSender>,
    pub extract_sender: Option<&'a ExtractSender>,
}

/// Outcome of a single-message pipeline run.
#[derive(Debug)]
pub enum ProcessOutcome {
    Stored {
        outcome: StoreOutcome,
        embedding_enqueued: bool,
    },
    Filtered {
        reason: String,
    },
    Errored {
        error: String,
    },
}

/// D-23: derive `actor_type` from a source string. Minimal heuristic — Phase 12 refines.
/// Any source containing "bot" => "bot"; otherwise "user".
pub fn derive_actor_type(source: &str) -> String {
    if source.to_ascii_lowercase().contains("bot") {
        "bot".to_string()
    } else {
        "user".to_string()
    }
}

/// Run the canonical per-message pipeline. See module-level docs for stage order.
pub async fn process_ingest_message<'a>(
    ctx: &ProcessMessageContext<'a>,
    input: ProcessMessageInput<'a>,
) -> ProcessOutcome {
    // Stage 1: Redaction — fail-closed (T-24.5-05). MUST run before tracing on content.
    let redacted_content = if let Some(engine) = ctx.redaction_engine {
        match engine.redact(input.content) {
            Ok(result) => {
                if result.was_redacted {
                    tracing::warn!(
                        write_path = %input.write_path,
                        categories = ?result.categories,
                        count = result.redaction_count,
                        "ingest: content redacted"
                    );
                }
                result.content
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    write_path = %input.write_path,
                    "ingest: redaction failed — rejecting message (fail-closed)"
                );
                return ProcessOutcome::Errored {
                    error: format!("redaction failed: {e}"),
                };
            }
        }
    } else {
        input.content.to_string()
    };

    // Stage 2: ContentFilter — Drop verdict => Filtered.
    if let Some(cf) = ctx.content_filter {
        match cf.check(&redacted_content).await {
            Ok(FilterVerdict::Drop { reason }) => {
                tracing::debug!(
                    write_path = %input.write_path,
                    reason = %reason,
                    "ingest: dropped by content filter"
                );
                return ProcessOutcome::Filtered { reason };
            }
            Ok(FilterVerdict::Allow) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    write_path = %input.write_path,
                    "ingest: content filter error, proceeding (fail-open)"
                );
            }
        }
    }

    // Stage 3: Category classification is supplied by the caller (auto-store path).
    // Ingest path passes None — salience defaults to 2.5.
    let category = input.category;

    // Stage 4: Summarization — only for assistant role, fail-open if provider errors.
    let is_assistant = input.role == "assistant";
    let (store_content, is_summarized) = if is_assistant {
        if let Some(provider) = ctx.summarization_provider {
            match provider.summarize(&redacted_content).await {
                Ok(summary) => {
                    tracing::debug!(
                        write_path = %input.write_path,
                        original_len = redacted_content.len(),
                        summary_len = summary.len(),
                        "ingest: summarized assistant response"
                    );
                    (summary, true)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        write_path = %input.write_path,
                        "ingest: summarization failed, storing raw (fail-open)"
                    );
                    (redacted_content.clone(), false)
                }
            }
        } else {
            (redacted_content.clone(), false)
        }
    } else {
        (redacted_content.clone(), false)
    };

    // Stage 5: Build tags.
    let mut tags = input.base_tags.clone();
    if is_summarized {
        tags.push("summarized".to_string());
    }
    tags.push(format!("session:{}", input.session_id));
    tags.push(format!("project:{}", input.project));
    if let Some(ref cr) = category {
        tags.push(format!("category:{}", cr.category));
    }

    // Stage 6: Event time extraction.
    let now_utc = input.timestamp.unwrap_or_else(chrono::Utc::now);
    let (event_time, event_time_precision) =
        match extract_event_time(&store_content, input.birth_year, now_utc) {
            Some((et, precision)) => (Some(et), Some(precision.as_str().to_string())),
            None => (None, None),
        };

    // Stage 7: Build CreateMemory + store.
    let actor_type = match input.write_path {
        "auto_store" => "auto-store".to_string(),
        _ => derive_actor_type(input.source),
    };
    let type_hint = if is_summarized {
        "summary".to_string()
    } else if input.write_path == "auto_store" {
        "auto".to_string()
    } else {
        "auto".to_string()
    };

    let create = CreateMemory {
        content: store_content,
        type_hint,
        source: input.source.to_string(),
        tags: Some(tags),
        created_at: input.timestamp,
        actor: input.actor.clone(),
        actor_type,
        audience: "global".to_string(),
        idempotency_key: input.idempotency_key.clone(),
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time,
        event_time_precision,
        project: Some(input.project.to_string()),
        trust_level: Some(0.3),
        session_id: Some(input.session_id.to_string()),
        agent_role: None,
        write_path: Some(input.write_path.to_string()),
        knowledge_tier: None,
        source_ids: None,
        reply_to_id: input.reply_to_id.clone(),
    };

    let outcome = match ctx.store.store_with_outcome(create).await {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(
                error = %e,
                write_path = %input.write_path,
                "ingest: store failed"
            );
            return ProcessOutcome::Errored {
                error: e.to_string(),
            };
        }
    };

    // Stage 8: Post-store side effects — only on Created. A dedup hit returns
    // the already-enqueued embedding status; re-enqueueing would be wasteful.
    let embedding_enqueued = if let StoreOutcome::Created(ref memory) = outcome {
        let stability: f64 = match &category {
            Some(cr) if cr.action == "store-low" => 1.5,
            _ => 2.5,
        };
        if let Err(e) = ctx
            .store
            .upsert_salience(&memory.id, stability, 5.0, 0, None)
            .await
        {
            tracing::warn!(
                error = %e,
                memory_id = %memory.id,
                "ingest: failed to seed salience"
            );
        }

        let mut enqueued = false;
        if let Some(sender) = ctx.embed_sender {
            let text = build_embedding_text(
                &memory.content,
                memory.abstract_text.as_deref(),
                &memory.tags,
            );
            let send_res = sender.try_send(EmbeddingJob {
                memory_id: memory.id.clone(),
                text,
                attempt: 0,
                completion_tx: None,
                tier: "fast".to_string(),
            });
            enqueued = send_res.is_ok();
        }

        if let Some(sender) = ctx.extract_sender {
            let _ = sender.try_send(ExtractionJob {
                memory_id: memory.id.clone(),
                content: memory.content.clone(),
                attempt: 0,
            });
        }

        enqueued
    } else {
        false
    };

    ProcessOutcome::Stored {
        outcome,
        embedding_enqueued,
    }
}
