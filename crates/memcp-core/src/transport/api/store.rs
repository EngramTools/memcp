//! POST /v1/store — store a memory with optional sync-wait for embedding.
//!
//! Replicates cmd_store business logic: resource cap checks, temporal extraction,
//! salience seeding, and optional wait=true polling for embedding completion.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde_json::json;

use super::types::{error_json, RedactionInfo, StoreRequest};
use crate::embedding::build_embedding_text;
use crate::pipeline::temporal::extract_event_time;
use crate::store::{CreateMemory, Memory, MemoryStore};
use crate::transport::health::AppState;

/// POST /v1/store
///
/// Stores a memory and returns its ID + embedding_status.
/// Response matches `memcp store --json` output shape.
///
/// When `wait: true`, blocks until embedding completes (or sync_timeout_secs expires).
/// When `wait: false` (default), returns immediately with embedding_status="pending".
pub async fn store_handler(
    State(state): State<AppState>,
    Json(req): Json<StoreRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
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

    // Validate required field
    if req.content.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_json("content is required and must not be empty")),
        );
    }

    // Resource cap check: replicate cmd_store lines 251-266
    if let Some(max) = state.config.resource_caps.max_memories {
        match store.count_live_memories().await {
            Ok(count) => {
                let ratio = count as f64 / max as f64;
                let hard_cap = state.config.resource_limits.hard_cap_percent as f64 / 100.0;
                if ratio >= hard_cap {
                    return (StatusCode::BAD_REQUEST, Json(error_json(&format!(
                        "Resource cap exceeded — max_memories limit is {} (current: {}, hard_cap: {}%)",
                        max, count, state.config.resource_limits.hard_cap_percent
                    ))));
                }
                // Warning threshold — log it (HTTP callers see it in logs, not response body)
                let warn_threshold = state.config.resource_limits.warn_percent as f64 / 100.0;
                if ratio >= warn_threshold {
                    tracing::warn!(
                        current = count,
                        max = max,
                        pct = (ratio * 100.0).round(),
                        "Memory usage at {}% — approaching capacity",
                        (ratio * 100.0).round()
                    );
                }
            }
            Err(e) => {
                // Fail-open: log warning, allow store (matches cmd_store behavior)
                tracing::warn!(error = %e, "Failed to check memory count — proceeding anyway");
            }
        }
    }

    // Redact secrets/PII before any further processing (before content_filter, before embedding)
    let mut redaction_info: Option<RedactionInfo> = None;
    let content = if !req.skip_redaction {
        if let Some(ref engine) = state.redaction_engine {
            match engine.redact(&req.content) {
                Ok(result) => {
                    if result.was_redacted {
                        tracing::warn!(
                            categories = ?result.categories,
                            count = result.redaction_count,
                            "Content redacted"
                        );
                        redaction_info = Some(RedactionInfo {
                            count: result.redaction_count,
                            categories: result.categories,
                        });
                    }
                    result.content
                }
                Err(e) => {
                    tracing::error!(error = %e, "Redaction failed — rejecting store (fail-closed)");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(error_json("Store rejected: redaction error")),
                    );
                }
            }
        } else {
            req.content.clone()
        }
    } else {
        req.content.clone()
    };

    // Extract temporal event time from content.
    let temporal_result =
        extract_event_time(&content, state.config.user.birth_year, Utc::now());
    let (event_time, event_time_precision) = match temporal_result {
        Some((dt, precision)) => (Some(dt), Some(precision.as_str().to_string())),
        None => (None, None),
    };

    let input = CreateMemory {
        content,
        type_hint: req.type_hint,
        source: req.source,
        tags: req.tags,
        created_at: None,
        actor: req.actor,
        actor_type: req.actor_type,
        audience: req.audience,
        idempotency_key: req.idempotency_key,
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time,
        event_time_precision,
        project: req.project,
        trust_level: req.trust_level,
        session_id: req.session_id,
        agent_role: req.agent_role,
    };

    let memory = match store.store(input).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "store failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Store failed: {}", e))),
            );
        }
    };

    // Seed salience: explicit API stores get stability=3.0 (same as CLI explicit store).
    if let Err(e) = store.upsert_salience(&memory.id, 3.0, 5.0, 0, None).await {
        tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience");
    }

    // Enqueue embedding job
    if let Some(ref sender) = state.embed_sender {
        let text = build_embedding_text(&memory.content, &memory.tags);
        let _ = sender.try_send(crate::embedding::EmbeddingJob {
            memory_id: memory.id.clone(),
            text,
            attempt: 0,
            completion_tx: None,
            tier: "fast".to_string(),
        });
    }

    // Build initial response
    let mut response_json = format_memory_json(&memory);
    if let Some(ref info) = redaction_info {
        if let serde_json::Value::Object(ref mut map) = response_json {
            map.insert(
                "redactions".to_string(),
                json!({ "count": info.count, "categories": info.categories }),
            );
        }
    }

    // wait=true: poll embedding_status until complete or timeout
    if req.wait {
        let timeout = std::time::Duration::from_secs(state.config.store.sync_timeout_secs);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= timeout {
                // Timeout — embedding still pending
                if let serde_json::Value::Object(ref mut map) = response_json {
                    map.insert("embedding_status".to_string(), json!("pending"));
                }
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match store.get(&memory.id).await {
                Ok(m) => {
                    if m.embedding_status == "complete" || m.embedding_status == "failed" {
                        if let serde_json::Value::Object(ref mut map) = response_json {
                            map.insert("embedding_status".to_string(), json!(m.embedding_status));
                        }
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    (StatusCode::OK, Json(response_json))
}

/// Format a stored Memory as JSON (compact, same as CLI non-verbose format).
fn format_memory_json(memory: &Memory) -> serde_json::Value {
    let mut obj = json!({
        "id": memory.id,
        "content": memory.content,
        "type_hint": memory.type_hint,
        "source": memory.source,
        "tags": memory.tags,
        "created_at": memory.created_at.to_rfc3339(),
        "actor": memory.actor,
        "actor_type": memory.actor_type,
        "audience": memory.audience,
        "embedding_status": memory.embedding_status,
    });
    if let Some(ref et) = memory.event_time {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("event_time".to_string(), json!(et.to_rfc3339()));
        }
    }
    if let Some(ref etp) = memory.event_time_precision {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("event_time_precision".to_string(), json!(etp));
        }
    }
    if let Some(ref ws) = memory.project {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("project".to_string(), json!(ws));
        }
    }
    obj
}
