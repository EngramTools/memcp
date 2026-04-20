//! HTTP API router — /v1/* routes for core memcp operations.
//!
//! All routes share `AppState` from `transport/health`. The router is merged
//! into the health server in `health::serve()`, enabling auth middleware to
//! be layered on `/v1/*` in Phase 12 without affecting /health or /status.

pub mod annotate;
pub mod auth;
pub mod batch_get;
pub mod delete;
pub mod discover;
pub mod entities;
pub mod export;
pub mod graph;
pub mod ingest;
pub mod memory_span;
pub mod pipeline;
pub mod recall;
pub mod search;
pub mod store;
pub mod types;
pub mod update;

use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::GlobalKeyExtractor, GovernorError,
    GovernorLayer,
};

use crate::config::RateLimitConfig;
use crate::transport::api::auth::AuthState;
use crate::transport::health::AppState;

/// Build a GovernorLayer with a global (non-per-IP) rate limit and JSON 429 responses.
///
/// `rps` = token replenishment rate (requests per second).
/// `burst_multiplier` = burst capacity = rps × multiplier.
///
/// Uses `GlobalKeyExtractor` — all callers share a single shared bucket per endpoint,
/// providing server-wide throughput protection.
/// 429 responses include `Retry-After` header and `{"error":"rate limited","retry_after_ms":N}`.
fn build_rate_limit_layer(
    rps: u32,
    burst_multiplier: u32,
) -> GovernorLayer<
    GlobalKeyExtractor,
    ::governor::middleware::StateInformationMiddleware,
    axum::body::Body,
> {
    use axum::body::Body;
    use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
    use axum::http::{Response as HttpResponse, StatusCode};

    let rps = rps.max(1);
    let burst = (rps * burst_multiplier).max(1);

    let config = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(GlobalKeyExtractor)
            .per_second(rps as u64)
            .burst_size(burst)
            .use_headers()
            .finish()
            .expect("valid governor config"),
    );

    GovernorLayer::new(config).error_handler(|err| {
        let (wait_secs, retry_header) = match &err {
            GovernorError::TooManyRequests { wait_time, .. } => {
                let s = *wait_time;
                (s, format!("{s}"))
            }
            _ => (1, "1".to_string()),
        };

        let body_str = format!(
            r#"{{"error":"rate limited","retry_after_ms":{}}}"#,
            wait_secs * 1000
        );

        let mut resp = HttpResponse::new(Body::from(body_str));
        *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
        if let Ok(val) = retry_header.parse() {
            resp.headers_mut().insert(RETRY_AFTER, val);
        }
        if let Ok(val) = "application/json".parse() {
            resp.headers_mut().insert(CONTENT_TYPE, val);
        }
        resp
    })
}

/// Build the /v1/* API router with per-endpoint rate limits.
///
/// Routes:
///   POST   /v1/recall                       — recall memories with optional query embedding
///   POST   /v1/search                       — hybrid search with salience re-ranking
///   POST   /v1/store                        — store a memory (with optional wait=true sync embedding)
///   POST   /v1/annotate                     — modify tags and/or salience on an existing memory
///   POST   /v1/update                       — replace memory content or metadata in place
///   DELETE /v1/memories/{id}               — hard delete a memory by ID
///   GET    /v1/status                       — alias for /status (convenience for plugin callers)
///   GET    /v1/export                       — export memories (jsonl, csv, markdown)
///   POST   /v1/discover                     — cosine sweet-spot discovery (creative association)
///   GET    /v1/entities                     — list knowledge graph entities
///   GET    /v1/entities/:id                 — single entity with facts + mention count
///   GET    /v1/entities/:id/relationships   — entity neighbor graph with traversal depth
///   GET    /v1/entities/:id/contradictions  — contradiction scan for an entity
///   GET    /v1/graph                        — full subgraph for dashboard visualization
///   GET    /v1/pipeline/health              — extraction + normalization pipeline status counts
///
/// Phase 12 pattern:
/// ```rust
/// let api_routes = api::router(&rl_config).layer(jwt_middleware);
/// ```
pub fn router(rl: &RateLimitConfig, auth_state: AuthState) -> Router<AppState> {
    if !rl.enabled {
        // Rate limiting disabled — flat router with no layers. /v1/ingest still needs
        // the auth layer so missing keys get 401 instead of being silently accepted on
        // a non-loopback bind (the boot-safety gate already refuses such configs, but
        // defense-in-depth is cheap). When `api_key` is None the middleware is a
        // passthrough, so loopback dev still works.
        let ingest_route = Router::new()
            .route("/v1/ingest", post(ingest::ingest_handler))
            .layer(axum::middleware::from_fn_with_state(
                auth_state.clone(),
                auth::require_api_key,
            ));
        return Router::new()
            .route("/v1/recall", post(recall::recall_handler))
            .route("/v1/search", post(search::search_handler))
            .route("/v1/store", post(store::store_handler))
            .route("/v1/annotate", post(annotate::annotate_handler))
            .route("/v1/update", post(update::update_handler))
            .route("/v1/memories/{id}", delete(delete::handle_delete))
            .route("/v1/memories/get", post(batch_get::handle_batch_get))
            .route("/v1/status", get(crate::transport::health::status_handler))
            .route("/v1/export", get(export::export_handler))
            .route("/v1/discover", post(discover::discover_handler))
            .route("/v1/entities", get(entities::list_entities_handler))
            .route("/v1/entities/{id}", get(entities::get_entity_handler))
            .route(
                "/v1/entities/{id}/relationships",
                get(entities::get_entity_relationships_handler),
            )
            .route(
                "/v1/entities/{id}/contradictions",
                get(entities::get_entity_contradictions_handler),
            )
            .route("/v1/graph", get(graph::graph_handler))
            .route("/v1/pipeline/health", get(pipeline::pipeline_health_handler))
            .route(
                "/v1/memory/span",
                post(memory_span::memory_span_handler),
            )
            .merge(ingest_route)
            .layer(DefaultBodyLimit::max(256 * 1024)); // 256KB hard limit on request bodies
    }

    let recall_routes = Router::new()
        .route("/v1/recall", post(recall::recall_handler))
        .layer(build_rate_limit_layer(rl.recall_rps, rl.burst_multiplier));

    let search_routes = Router::new()
        .route("/v1/search", post(search::search_handler))
        .layer(build_rate_limit_layer(rl.search_rps, rl.burst_multiplier));

    let store_routes = Router::new()
        .route("/v1/store", post(store::store_handler))
        .layer(build_rate_limit_layer(rl.store_rps, rl.burst_multiplier));

    let annotate_routes = Router::new()
        .route("/v1/annotate", post(annotate::annotate_handler))
        .layer(build_rate_limit_layer(rl.annotate_rps, rl.burst_multiplier));

    let update_routes = Router::new()
        .route("/v1/update", post(update::update_handler))
        .layer(build_rate_limit_layer(rl.update_rps, rl.burst_multiplier));

    let discover_routes = Router::new()
        .route("/v1/discover", post(discover::discover_handler))
        .layer(build_rate_limit_layer(rl.discover_rps, rl.burst_multiplier));

    let delete_routes = Router::new()
        .route("/v1/memories/{id}", delete(delete::handle_delete))
        .layer(build_rate_limit_layer(rl.delete_rps, rl.burst_multiplier));

    let export_routes = Router::new()
        .route("/v1/export", get(export::export_handler))
        .layer(build_rate_limit_layer(rl.export_rps, rl.burst_multiplier));

    let batch_get_routes = Router::new()
        .route("/v1/memories/get", post(batch_get::handle_batch_get))
        .layer(build_rate_limit_layer(
            rl.batch_get_rps,
            rl.burst_multiplier,
        ));

    // Entity graph routes share the search rate limit — read-only queries
    // with similar cost profile to hybrid search.
    let entity_routes = Router::new()
        .route("/v1/entities", get(entities::list_entities_handler))
        .route("/v1/entities/{id}", get(entities::get_entity_handler))
        .route(
            "/v1/entities/{id}/relationships",
            get(entities::get_entity_relationships_handler),
        )
        .route(
            "/v1/entities/{id}/contradictions",
            get(entities::get_entity_contradictions_handler),
        )
        .layer(build_rate_limit_layer(rl.search_rps, rl.burst_multiplier));

    let graph_routes = Router::new()
        .route("/v1/graph", get(graph::graph_handler))
        .layer(build_rate_limit_layer(rl.search_rps, rl.burst_multiplier));

    let pipeline_routes = Router::new()
        .route("/v1/pipeline/health", get(pipeline::pipeline_health_handler))
        .layer(build_rate_limit_layer(rl.search_rps, rl.burst_multiplier));

    // Phase 24.75 Plan 04 (CHUNK-04): topic-span drill-down. Per D-04 + Pattern A4
    // (RESEARCH), /v1/memory/span is a read-only endpoint — no auth layer, rate-limit
    // only (mirrors /v1/search). Embedding cost per call is O(N_spans) so it shares
    // the search rate bucket.
    let memory_span_routes = Router::new()
        .route(
            "/v1/memory/span",
            post(memory_span::memory_span_handler),
        )
        .layer(build_rate_limit_layer(rl.search_rps, rl.burst_multiplier));

    // CRITICAL: auth is .layer()'d LAST so it wraps OUTERMOST and runs FIRST.
    // `.layer(A).layer(B)` => request flow is B -> A -> handler, so auth must be
    // the second `.layer(...)` to gate rate-limit token consumption on authenticated
    // callers only. See RESEARCH pitfall 1 — reversing these burns rate-limit tokens
    // on unauthenticated calls.
    let ingest_routes = Router::new()
        .route("/v1/ingest", post(ingest::ingest_handler))
        .layer(build_rate_limit_layer(rl.ingest_rps, rl.burst_multiplier))
        .layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            auth::require_api_key,
        ));

    Router::new()
        .merge(recall_routes)
        .merge(search_routes)
        .merge(store_routes)
        .merge(annotate_routes)
        .merge(update_routes)
        .merge(discover_routes)
        .merge(delete_routes)
        .merge(export_routes)
        .merge(batch_get_routes)
        .merge(entity_routes)
        .merge(graph_routes)
        .merge(pipeline_routes)
        .merge(memory_span_routes)
        .merge(ingest_routes)
        .route("/v1/status", get(crate::transport::health::status_handler))
        .layer(DefaultBodyLimit::max(256 * 1024)) // 256KB hard limit on request bodies
}
