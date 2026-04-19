//! POST /v1/ingest handler + shared ingest pipeline entry (Phase 24.5 Plan 03).
//!
//! Delegates the per-message body to `pipeline::auto_store::shared::process_ingest_message`,
//! which enforces the D-10 stage order (redaction -> content_filter -> summarize ->
//! store_with_outcome -> salience -> embedding enqueue). The handler owns:
//!   - Batch size / per-message size validation
//!   - Within-batch reply chain (D-17) + caller override (D-18)
//!   - Idempotency key defaulting via `make_idempotency_key` (D-13)
//!   - StoreOutcome -> "stored" | "duplicate" status mapping (D-14)
//!   - Per-batch aggregate summary counts

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use sha2::{Digest, Sha256};

use super::types::{error_json, IngestRequest, IngestResult, IngestSummary};
use crate::pipeline::auto_store::shared::{
    process_ingest_message, ProcessMessageContext, ProcessMessageInput, ProcessOutcome,
};
use crate::store::StoreOutcome;
use crate::transport::health::AppState;

/// D-13: Deterministic SHA-256 idempotency key over (source, session_id, timestamp, role, content).
///
/// Fields are length-prefixed (LE u32) before hashing so that `(source="ab", session="c")`
/// and `(source="a", session="bc")` cannot collide via boundary ambiguity (RESEARCH Topic 2).
/// Stable across daemon restarts and across Rust compiler versions.
pub fn make_idempotency_key(
    source: &str,
    session_id: &str,
    timestamp: chrono::DateTime<chrono::Utc>,
    role: &str,
    content: &str,
) -> String {
    let mut hasher = Sha256::new();
    for field in &[source, session_id, role, content] {
        hasher.update((field.len() as u32).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    let ts = timestamp.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    hasher.update((ts.len() as u32).to_le_bytes());
    hasher.update(ts.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// POST /v1/ingest
///
/// Accepts a batch of conversation turns. Processes each message sequentially
/// through the auto-store-parity pipeline. Returns 200 with per-message results
/// even when individual messages fail (D-08 best-effort semantics).
pub async fn ingest_handler(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !state.ready.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_json("daemon not ready")),
        );
    }
    if state.store.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(error_json("store not available")),
        );
    }

    let (status, body) = run_ingest_batch(&state, req).await;
    (status, Json(body))
}

/// Core batch-processing path. Exposed for MCP / CLI callers (Plan 24.5-04) that do
/// NOT come through axum extractors but still need the identical per-message semantics.
pub async fn run_ingest_batch(
    state: &AppState,
    req: IngestRequest,
) -> (StatusCode, serde_json::Value) {
    // Batch-size guard (D-20 / T-24.5-03).
    let max_batch = state.config.ingest.max_batch_size;
    if req.messages.len() > max_batch {
        return (
            StatusCode::BAD_REQUEST,
            error_json(&format!(
                "batch size {} exceeds configured max_batch_size {}",
                req.messages.len(),
                max_batch
            )),
        );
    }

    let store = match state.store.as_ref() {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                error_json("store not available"),
            )
        }
    };

    let helper_ctx = ProcessMessageContext {
        store,
        redaction_engine: state.redaction_engine.as_deref(),
        content_filter: state.content_filter.as_ref(),
        summarization_provider: state.summarization_provider.as_ref(),
        embed_sender: state.embed_sender.as_ref(),
        extract_sender: state.extract_sender.as_ref(),
    };

    let max_content = state.config.ingest.max_content_size;
    let total = req.messages.len();

    let mut results: Vec<IngestResult> = Vec::with_capacity(total);
    let mut summary = IngestSummary::default();
    // D-17: track last successfully stored memory id so msg[N+1] chains to msg[N].
    // Per RESEARCH Topic 4: on error / filter, prev_id is NOT advanced — next message
    // chains to the last successful memory (reflects what actually exists in DB).
    let mut prev_id: Option<String> = None;

    for (idx, msg) in req.messages.iter().enumerate() {
        // Per-message content-size guard (T-24.5-03).
        if msg.content.len() > max_content {
            summary.errored += 1;
            results.push(IngestResult {
                index: idx,
                status: "error",
                memory_id: None,
                reason: Some(format!(
                    "content too large: {} bytes exceeds max_content_size {}",
                    msg.content.len(),
                    max_content
                )),
                embedding_status: None,
            });
            continue;
        }

        // D-18 > D-17: caller override wins; else chain to previous successful id.
        let effective_reply_to = msg
            .reply_to_id
            .clone()
            .or_else(|| prev_id.clone());

        let timestamp = msg.timestamp.unwrap_or_else(chrono::Utc::now);
        let idempotency_key = msg.idempotency_key.clone().or_else(|| {
            Some(make_idempotency_key(
                &req.source,
                &req.session_id,
                timestamp,
                &msg.role,
                &msg.content,
            ))
        });

        let input = ProcessMessageInput {
            source: req.source.as_str(),
            session_id: req.session_id.as_str(),
            project: req.project.as_str(),
            role: msg.role.as_str(),
            content: msg.content.as_str(),
            timestamp: Some(timestamp),
            idempotency_key,
            reply_to_id: effective_reply_to,
            actor: None,
            write_path: "ingest",
            base_tags: Vec::new(),
            category: None,
            birth_year: state.config.user.birth_year,
        };

        let outcome = process_ingest_message(&helper_ctx, input).await;

        match outcome {
            ProcessOutcome::Stored {
                outcome: StoreOutcome::Created(memory),
                embedding_enqueued,
            } => {
                prev_id = Some(memory.id.clone());
                summary.stored += 1;
                tracing::debug!(
                    batch_index = idx,
                    memory_id = %memory.id,
                    "ingest: stored"
                );
                results.push(IngestResult {
                    index: idx,
                    status: "stored",
                    memory_id: Some(memory.id),
                    reason: None,
                    embedding_status: if embedding_enqueued { Some("pending") } else { None },
                });
            }
            ProcessOutcome::Stored {
                outcome: StoreOutcome::Deduplicated(memory),
                ..
            } => {
                prev_id = Some(memory.id.clone());
                summary.duplicate += 1;
                tracing::debug!(
                    batch_index = idx,
                    memory_id = %memory.id,
                    "ingest: duplicate"
                );
                results.push(IngestResult {
                    index: idx,
                    status: "duplicate",
                    memory_id: Some(memory.id),
                    reason: None,
                    embedding_status: None,
                });
            }
            ProcessOutcome::Filtered { reason } => {
                summary.filtered += 1;
                tracing::debug!(batch_index = idx, reason = %reason, "ingest: filtered");
                results.push(IngestResult {
                    index: idx,
                    status: "filtered",
                    memory_id: None,
                    reason: Some(reason),
                    embedding_status: None,
                });
            }
            ProcessOutcome::Errored { error } => {
                summary.errored += 1;
                tracing::debug!(batch_index = idx, error = %error, "ingest: errored");
                results.push(IngestResult {
                    index: idx,
                    status: "error",
                    memory_id: None,
                    reason: Some(error),
                    embedding_status: None,
                });
            }
        }
    }

    tracing::info!(
        total = total,
        stored = summary.stored,
        filtered = summary.filtered,
        duplicate = summary.duplicate,
        errored = summary.errored,
        "ingest: batch complete"
    );

    (
        StatusCode::OK,
        serde_json::json!({
            "results": results,
            "summary": summary,
        }),
    )
}
