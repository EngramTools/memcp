//! POST /v1/discover — cosine sweet-spot discovery for creative association.
//!
//! Finds memories in the similarity middle ground (default 0.3-0.7) — related
//! enough to be meaningful but different enough to be surprising.
//! Use for creative exploration and lateral thinking, not exact retrieval.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use metrics;
use serde::Deserialize;
use serde_json::json;

use super::types::error_json;
use crate::transport::health::AppState;

#[derive(Deserialize)]
pub struct DiscoverParams {
    /// Topic or concept to explore connections for
    pub query: String,
    /// Minimum cosine similarity (default 0.3). Lower = more surprising connections.
    pub min_similarity: Option<f64>,
    /// Maximum cosine similarity (default 0.7). Higher = more obviously related.
    pub max_similarity: Option<f64>,
    /// Maximum results (default 10)
    pub limit: Option<u32>,
    /// Project scope filter
    pub project: Option<String>,
}

/// POST /v1/discover
///
/// Returns memories in the cosine sweet spot — related but not identical to the query.
/// Requires daemon to be running (embedding + PostgreSQL store).
pub async fn discover_handler(
    State(state): State<AppState>,
    Json(params): Json<DiscoverParams>,
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

    if params.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_json("query is required and must not be empty")),
        );
    }

    let min_sim = params.min_similarity.unwrap_or(0.3).clamp(0.0, 1.0);
    let max_sim = params.max_similarity.unwrap_or(0.7).clamp(0.0, 1.0);
    let limit = params.limit.unwrap_or(10).clamp(1, 50);

    if min_sim >= max_sim {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_json(
                "min_similarity must be less than max_similarity",
            )),
        );
    }

    // Embed query
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

    let embedding = match provider.embed(&params.query).await {
        Ok(emb) => pgvector::Vector::from(emb),
        Err(e) => {
            tracing::warn!(error = %e, "embedding failed in discover handler");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Embedding failed: {}", e))),
            );
        }
    };

    // Run discovery
    let results = match store
        .discover_associations(
            &embedding,
            min_sim,
            max_sim,
            limit,
            params.project.as_deref(),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "discover_associations failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Discovery failed: {}", e))),
            );
        }
    };

    // Build response array
    let discoveries: Vec<serde_json::Value> = results
        .iter()
        .map(|(memory, sim)| {
            json!({
                "id": memory.id,
                "content": memory.content,
                "type_hint": memory.type_hint,
                "tags": memory.tags,
                "similarity": format!("{:.3}", sim),
                "created_at": memory.created_at.to_rfc3339(),
                "project": memory.project,
            })
        })
        .collect();

    let count = discoveries.len();
    metrics::histogram!("memcp_discover_results_returned").record(count as f64);
    let output = json!({
        "discoveries": discoveries,
        "query": params.query,
        "similarity_range": [min_sim, max_sim],
        "count": count,
    });

    (StatusCode::OK, Json(output))
}
