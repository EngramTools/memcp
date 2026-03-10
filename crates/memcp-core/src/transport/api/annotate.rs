//! POST /v1/annotate — modify tags and/or salience on an existing memory.
//!
//! Delegates to the shared `annotate_logic()` helper in cli.rs.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use serde_json::json;

use super::types::{error_json, AnnotateRequest};
use crate::cli::annotate_logic;
use crate::transport::health::AppState;

/// POST /v1/annotate
///
/// Appends or replaces tags and/or adjusts salience on an existing memory.
/// Returns the diff: which tags were added/removed and salience before/after.
pub async fn annotate_handler(
    State(state): State<AppState>,
    Json(req): Json<AnnotateRequest>,
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

    if req.id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_json("id is required and must not be empty")),
        );
    }

    match annotate_logic(&store, &req.id, req.tags, req.replace_tags, req.salience).await {
        Ok(result) => {
            let mut changes = serde_json::Map::new();
            changes.insert("tags_added".to_string(), json!(result.tags_added));
            changes.insert("tags_removed".to_string(), json!(result.tags_removed));
            if let (Some(before), Some(after)) = (result.salience_before, result.salience_after) {
                changes.insert("salience_before".to_string(), json!(before));
                changes.insert("salience_after".to_string(), json!(after));
            }
            let output = json!({
                "id": result.id,
                "changes": changes,
            });
            (StatusCode::OK, Json(output))
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Memory not found") || msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(error_json(&format!("Memory not found: {}", req.id))),
                )
            } else {
                tracing::warn!(error = %e, memory_id = %req.id, "annotate failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json(&format!("Annotate failed: {}", e))),
                )
            }
        }
    }
}
