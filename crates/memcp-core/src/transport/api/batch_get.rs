//! POST /v1/memories/get — batch retrieve full memory content by ID.
//!
//! Returns full untruncated content for each valid ID. Missing IDs are
//! returned in a `not_found` array. Request capped at 50 IDs.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use serde_json::json;

use super::types::{error_json, BatchGetRequest};
use crate::store::MemoryStore;
use crate::transport::health::AppState;

/// Maximum number of IDs allowed in a single batch request.
const MAX_BATCH_SIZE: usize = 50;

/// A single memory in the batch response — only consumer-useful fields.
#[derive(Serialize)]
pub struct BatchMemory {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub salience: f64,
    pub created_at: String,
    pub type_hint: String,
}

/// POST /v1/memories/get
///
/// Batch retrieve full memory content by ID. Each retrieved memory is
/// touched (access_count++, last_accessed_at updated) via store.get().
///
/// Returns:
///   200 OK — with `memories` array and `not_found` array
///   413 Payload Too Large — if more than 50 IDs requested
///   503 Service Unavailable — daemon not ready or store unavailable
pub async fn handle_batch_get(
    State(state): State<AppState>,
    Json(req): Json<BatchGetRequest>,
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

    if req.ids.len() > MAX_BATCH_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(error_json(&format!(
                "Too many IDs: {} exceeds maximum of {}",
                req.ids.len(),
                MAX_BATCH_SIZE
            ))),
        );
    }

    let mut memories = Vec::new();
    let mut not_found = Vec::new();

    for id in &req.ids {
        if id.trim().is_empty() {
            not_found.push(id.clone());
            continue;
        }

        match store.get(id).await {
            Ok(memory) => {
                // Convert tags from Option<serde_json::Value> (JSONB array) to Vec<String>
                let tags: Vec<String> = memory
                    .tags
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();

                memories.push(BatchMemory {
                    id: memory.id,
                    content: memory.content,
                    tags,
                    salience: memory.access_count as f64, // raw access count as salience proxy
                    created_at: memory.created_at.to_rfc3339(),
                    type_hint: memory.type_hint,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("NotFound") {
                    not_found.push(id.clone());
                } else {
                    tracing::warn!(error = %e, memory_id = %id, "batch get failed for memory");
                    not_found.push(id.clone());
                }
            }
        }
    }

    let memories_val = serde_json::to_value(&memories).unwrap_or(json!([]));
    let not_found_val = serde_json::to_value(&not_found).unwrap_or(json!([]));

    (
        StatusCode::OK,
        Json(json!({
            "memories": memories_val,
            "not_found": not_found_val,
        })),
    )
}
