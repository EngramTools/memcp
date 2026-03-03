//! DELETE /v1/memories/{id} — hard delete a memory by ID.
//!
//! Returns 204 No Content on success, 404 if the memory does not exist.
//! Uses the store's `get()` to check existence (get() returns NotFound for
//! deleted memories), then hard-deletes with `delete()`.

use std::sync::atomic::Ordering;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;

use crate::store::MemoryStore;
use crate::transport::health::AppState;
use super::types::error_json;

/// DELETE /v1/memories/{id}
///
/// Hard-deletes a memory from the store. The memory is permanently removed
/// from Postgres — this is not a soft delete.
///
/// Returns:
///   204 No Content — memory deleted successfully
///   404 Not Found  — memory does not exist (or was already deleted)
///   503 Service Unavailable — daemon not ready or store unavailable
pub async fn handle_delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !state.ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("daemon not ready")));
    }

    let store = match &state.store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("store not available"))),
    };

    if id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(error_json("id is required")));
    }

    // Check existence first — get() returns NotFound for deleted/missing memories.
    match store.get(&id).await {
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("NotFound") {
                return (StatusCode::NOT_FOUND, Json(error_json(&format!("Memory not found: {}", id))));
            }
            tracing::warn!(error = %e, memory_id = %id, "delete existence check failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_json(&format!("Delete failed: {}", e))));
        }
        Ok(_) => {} // memory exists — proceed with delete
    }

    match store.delete(&id).await {
        Ok(()) => (StatusCode::NO_CONTENT, Json(json!({}))),
        Err(e) => {
            tracing::warn!(error = %e, memory_id = %id, "delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_json(&format!("Delete failed: {}", e))))
        }
    }
}
