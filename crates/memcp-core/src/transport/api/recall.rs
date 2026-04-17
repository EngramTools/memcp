//! POST /v1/recall — execute recall with optional query embedding.
//!
//! Supports both query-based recall (vector similarity) and queryless recall
//! (salience-ranked). Injects project summary and preamble when first=true.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde_json::json;

use super::types::{error_json, RecallRequest};
use crate::recall::RecallEngine;
use crate::transport::health::AppState;

/// Default preamble text shown to agents on first=true.
const DEFAULT_PREAMBLE: &str = "You have access to persistent memory via memcp. \
    Key commands: `memcp store \"content\" --tags tag1,tag2` to save, \
    `memcp search \"query\"` to find, `memcp get <id>` for full content, \
    `memcp annotate --id <id> --tags tag1 --salience 1.5x` to enrich. \
    Memories persist across sessions. Store important decisions, preferences, and context.";

/// POST /v1/recall
///
/// Executes recall and returns the same JSON shape as `memcp recall --json`.
/// When `query` is absent or empty, uses the queryless path (salience-ranked, no embedding).
/// When `query` is present, embeds via the in-process embed_provider (no IPC needed).
pub async fn recall_handler(
    State(state): State<AppState>,
    Json(req): Json<RecallRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Readiness gate
    if !state.ready.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_json("daemon not ready")),
        );
    }

    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(error_json("store not available")),
            )
        }
    };

    let engine = RecallEngine::new(
        store.clone(),
        state.config.recall.clone(),
        state.config.extraction.enabled,
    );

    let query_str = req.query.as_deref().unwrap_or("").trim().to_string();
    let is_queryless = query_str.is_empty();

    let mut result = if is_queryless {
        // Queryless path — no embedding needed; ranked by salience + recency.
        match engine
            .recall_queryless(
                req.session_id,
                req.reset,
                req.project.as_deref(),
                req.first,
                req.limit,
                &req.boost_tags,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "recall_queryless failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json(&format!("Recall failed: {}", e))),
                );
            }
        }
    } else {
        // Query-based path — embed in-process via embed_provider.
        let provider = match &state.embed_provider {
            Some(p) => p.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(error_json(
                        "embedding provider not available — daemon not fully initialized",
                    )),
                )
            }
        };

        let query_embedding = match provider.embed(&query_str).await {
            Ok(emb) => emb,
            Err(e) => {
                tracing::warn!(error = %e, "embedding failed in recall handler");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json(&format!("Embedding failed: {}", e))),
                );
            }
        };

        match engine
            .recall(
                &query_embedding,
                req.session_id,
                req.reset,
                req.project.as_deref(),
                &req.boost_tags,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "recall failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json(&format!("Recall failed: {}", e))),
                );
            }
        }
    };

    // For query-based path with first=true, fetch project summary separately.
    // (recall_queryless already handles this internally via the first parameter.)
    if !is_queryless && req.first && result.summary.is_none() {
        result.summary = match store.fetch_project_summary(req.project.as_deref()).await {
            Ok(Some((id, content))) => Some(crate::recall::RecalledMemory {
                memory_id: id,
                content,
                relevance: 1.0,
                boost_applied: false,
                boost_score: 0.0,
                trust_level: 1.0,
                abstract_text: None,
                overview_text: None,
            }),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(error = %e, "fetch_project_summary failed — skipping");
                None
            }
        };
    }

    // Fetch related context if enabled and there are memories.
    let memory_ids: Vec<String> = result
        .memories
        .iter()
        .map(|m| m.memory_id.clone())
        .collect();
    let related_map = if state.config.recall.related_context_enabled && !memory_ids.is_empty() {
        match store.get_related_context(&memory_ids).await {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "get_related_context failed — skipping related hints");
                std::collections::HashMap::new()
            }
        }
    } else {
        std::collections::HashMap::new()
    };

    let truncation_chars = state.config.recall.truncation_chars;

    // Record histogram of memories returned per recall request.
    metrics::histogram!("memcp_recall_memories_returned").record(result.memories.len() as f64);

    // Build memories array with truncation and related context.
    let depth = req.depth;
    let memories: Vec<serde_json::Value> = result
        .memories
        .iter()
        .map(|mem| {
            let source_content = match depth {
                0 => mem.abstract_text.as_deref().unwrap_or(&mem.content),
                1 => mem.overview_text.as_deref().unwrap_or(&mem.content),
                _ => &mem.content,
            };
            let (truncated_content, was_truncated) =
                truncate_content(source_content, truncation_chars);

            let mut obj = json!({
                "id": mem.memory_id,
                "content": truncated_content,
                "relevance": mem.relevance,
            });

            if was_truncated {
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert("truncated".to_string(), json!(true));
                }
            }

            if mem.boost_applied {
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert("boost_applied".to_string(), json!(true));
                    map.insert("boost_score".to_string(), json!(mem.boost_score));
                }
            }

            if let Some(related) = related_map.get(&mem.memory_id) {
                if related.related_count > 0 {
                    let hint = build_related_hint(&related.shared_tags);
                    if let serde_json::Value::Object(ref mut map) = obj {
                        map.insert("related_count".to_string(), json!(related.related_count));
                        if !hint.is_empty() {
                            map.insert("hint".to_string(), json!(hint));
                        }
                    }
                }
            }

            obj
        })
        .collect();

    // Attach source chain if show_sources is requested (D-08: opt-in).
    let mut memories = memories;
    if req.show_sources {
        let mem_ids: Vec<String> = result.memories.iter().map(|m| m.memory_id.clone()).collect();
        if let Ok(full_memories) = store.get_memories_by_ids(&mem_ids).await {
            for (i, recalled) in result.memories.iter().enumerate() {
                if let Some(full_mem) = full_memories.get(&recalled.memory_id) {
                    let sources = if req.show_sources_deep {
                        crate::transport::server::fetch_source_chain_deep(&store, full_mem).await
                    } else {
                        crate::transport::server::fetch_source_chain_single_hop(&store, full_mem).await
                    };
                    if !sources.is_empty() {
                        let source_entries: Vec<serde_json::Value> = sources
                            .iter()
                            .map(|(id, mem)| {
                                json!({
                                    "id": id,
                                    "content": crate::transport::server::truncate_source_content(&mem.content, 200),
                                    "knowledge_tier": mem.knowledge_tier,
                                })
                            })
                            .collect();
                        if let Some(obj) = memories.get_mut(i).and_then(|v| v.as_object_mut()) {
                            obj.insert("sources".to_string(), json!(source_entries));
                        }
                    }
                }
            }
        }
    }

    // Assemble final output — same shape as CLI --json output.
    let mut output = json!({
        "session_id": result.session_id,
        "count": result.count,
        "memories": memories,
    });

    if let Some(ref summary) = result.summary {
        if let serde_json::Value::Object(ref mut map) = output {
            map.insert(
                "summary".to_string(),
                json!({
                    "id": summary.memory_id,
                    "content": summary.content,
                }),
            );
        }
    }

    if req.first {
        let preamble = state
            .config
            .recall
            .preamble_override
            .as_deref()
            .unwrap_or(DEFAULT_PREAMBLE);
        if let serde_json::Value::Object(ref mut map) = output {
            map.insert(
                "current_datetime".to_string(),
                json!(Utc::now().to_rfc3339()),
            );
            map.insert("preamble".to_string(), json!(preamble));
        }
    }

    (StatusCode::OK, Json(output))
}

/// Truncate content to at most `max_chars` Unicode scalar values.
/// Returns `(truncated_content, was_truncated)`.
fn truncate_content(content: &str, max_chars: usize) -> (String, bool) {
    if content.chars().count() <= max_chars {
        (content.to_string(), false)
    } else {
        let truncated: String = content.chars().take(max_chars).collect();
        (format!("{}...", truncated), true)
    }
}

/// Build a ready-made `memcp search --tags ...` command from shared tags.
fn build_related_hint(shared_tags: &[String]) -> String {
    if shared_tags.is_empty() {
        return String::new();
    }
    format!("memcp search --tags {}", shared_tags.join(","))
}
