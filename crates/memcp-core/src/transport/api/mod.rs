//! HTTP API router — /v1/* routes for core memcp operations.
//!
//! All routes share `AppState` from `transport/health`. The router is merged
//! into the health server in `health::serve()`, enabling auth middleware to
//! be layered on `/v1/*` in Phase 12 without affecting /health or /status.

pub mod types;
pub mod recall;
pub mod search;
pub mod store;
pub mod annotate;
pub mod update;

use axum::{Router, routing::{get, post}};
use crate::transport::health::AppState;

/// Build the /v1/* API router.
///
/// Routes:
///   POST /v1/recall   — recall memories with optional query embedding
///   POST /v1/search   — hybrid search with salience re-ranking
///   POST /v1/store    — store a memory (with optional wait=true sync embedding)
///   POST /v1/annotate — modify tags and/or salience on an existing memory
///   POST /v1/update   — replace memory content or metadata in place
///   GET  /v1/status   — alias for /status (convenience for plugin callers)
///
/// Phase 12 pattern:
/// ```rust
/// let api_routes = api::router().layer(jwt_middleware);
/// ```
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/recall", post(recall::recall_handler))
        .route("/v1/search", post(search::search_handler))
        .route("/v1/store", post(store::store_handler))
        .route("/v1/annotate", post(annotate::annotate_handler))
        .route("/v1/update", post(update::update_handler))
        .route("/v1/status", get(crate::transport::health::status_handler))
}
