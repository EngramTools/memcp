//! POST /v1/update — replace memory content or metadata in place.
//!
//! When content changes, resets embedding_status to pending so daemon re-embeds.
//! Mirrors cmd_update behavior from cli.rs.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use serde_json::json;

use crate::store::{UpdateMemory, Memory, MemoryStore};
use crate::transport::health::AppState;
use super::types::{UpdateRequest, error_json};

/// POST /v1/update
///
/// Updates a memory's content or metadata. Returns the updated memory JSON.
/// Content changes trigger embedding re-queuing (embedding_status reset to pending).
pub async fn update_handler(
    State(state): State<AppState>,
    Json(req): Json<UpdateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !state.ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("daemon not ready")));
    }

    let store = match &state.store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("store not available"))),
    };

    if req.id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(error_json("id is required and must not be empty")));
    }

    // Validate at least one field to update
    if req.content.is_none() && req.type_hint.is_none() && req.source.is_none() && req.tags.is_none() {
        return (StatusCode::BAD_REQUEST, Json(error_json("at least one field is required: content, type_hint, source, or tags")));
    }

    let has_content_change = req.content.is_some();

    let input = UpdateMemory {
        content: req.content,
        type_hint: req.type_hint,
        source: req.source,
        tags: req.tags,
        trust_level: None,
    };

    let memory: Memory = match store.update(&req.id, input).await {
        Ok(m) => m,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("no rows") || msg.contains("RowNotFound") {
                return (StatusCode::NOT_FOUND, Json(error_json(&format!("Memory not found: {}", req.id))));
            }
            tracing::warn!(error = %e, memory_id = %req.id, "update failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_json(&format!("Update failed: {}", e))));
        }
    };

    // Reset embedding_status when content changed so daemon re-embeds.
    // update() only updates fields in UpdateMemory — does NOT reset embedding_status.
    if has_content_change {
        if let Err(e) = store.update_embedding_status(&req.id, "pending").await {
            tracing::warn!(error = %e, memory_id = %req.id, "Failed to reset embedding_status — daemon will poll and re-embed");
            // Fail-open: don't abort the response
        }
    }

    (StatusCode::OK, Json(format_memory_json(&memory)))
}

/// Format an updated Memory as JSON (compact, same shape as CLI store/update output).
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
