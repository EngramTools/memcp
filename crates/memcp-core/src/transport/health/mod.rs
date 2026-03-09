//! Health HTTP server — axum-based liveness and status endpoints.
//!
//! Provides /health (liveness probe, sub-ms AtomicBool check) and /status
//! (component health + resource usage). Runs on separate configurable port.
//! Spawned by transport/daemon, queries storage/ for live metrics.

use axum::{Router, Json, extract::State, routing::get, http::StatusCode};
use metrics_exporter_prometheus::PrometheusHandle;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tower_http::trace::TraceLayer;

use crate::config::{Config, ResourceCapsConfig};
use crate::embedding::{EmbeddingProvider, EmbeddingJob};

/// Shared state for the health and API server.
///
/// Renamed from HealthState in Phase 08.12 to AppState — now carries embedding
/// pipeline access for HTTP API handlers (/v1/*). Health handlers carry a few
/// extra fields they don't use — acceptable for a private internal struct.
#[derive(Clone)]
pub struct AppState {
    /// Set to true after DB connect + migrations complete.
    pub ready: Arc<AtomicBool>,
    /// Startup instant for uptime calculation.
    pub started_at: Instant,
    /// Resource caps from config.
    pub caps: ResourceCapsConfig,
    /// Postgres store for live queries (/status and /v1/* handlers).
    pub store: Option<Arc<crate::store::postgres::PostgresMemoryStore>>,
    /// Full daemon config for API handlers (RecallConfig, ExtractionConfig, StoreConfig, etc).
    pub config: Arc<Config>,
    /// Embedding provider for in-process query embedding (recall + search handlers).
    /// None when daemon was unable to init the provider (handlers return 503).
    pub embed_provider: Option<Arc<dyn EmbeddingProvider + Send + Sync>>,
    /// Embedding pipeline sender for enqueuing jobs (store handler wait=true path).
    /// None when pipeline not available (store handler falls back to polling).
    pub embed_sender: Option<mpsc::Sender<EmbeddingJob>>,
    /// Prometheus scrape handle for /metrics endpoint.
    pub metrics_handle: PrometheusHandle,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

/// GET /health — liveness/readiness probe.
/// Returns 200 when ready, 503 when not ready.
async fn health_handler(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    if state.ready.load(Ordering::Acquire) {
        (StatusCode::OK, Json(HealthResponse { status: "ok" }))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(HealthResponse { status: "starting" }))
    }
}

/// GET /status — operational status with component health and resource usage.
/// Also exposed as pub for use as /v1/status alias.
pub async fn status_handler(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    let is_ready = state.ready.load(Ordering::Acquire);
    let uptime = state.started_at.elapsed().as_secs();

    // Check DB connectivity and gather resource counts + pool breakdown
    let (db_status, memory_count, db_conn_count, pool_active, pool_idle) = if let Some(ref store) = state.store {
        // Quick connectivity check via pool
        let pool = store.pool();
        let pool_size = pool.size() as u64;
        let idle = pool.num_idle() as u64;
        let active = pool_size.saturating_sub(idle);

        match store.count_live_memories().await {
            Ok(count) => ("ok", Some(count as u64), Some(pool_size), Some(active), Some(idle)),
            Err(_) => ("degraded", None, Some(pool_size), Some(active), Some(idle)),
        }
    } else {
        ("down", None, None, None, None)
    };

    // Pending embedding count — real-time backlog visible to operators
    let pending_embeddings: Option<i64> = if let Some(ref store) = state.store {
        store.count_pending_embeddings().await.ok()
    } else {
        None
    };

    // Check HNSW index presence
    let hnsw_status = if let Some(ref store) = state.store {
        match sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pg_indexes WHERE indexname = 'idx_memory_embeddings_hnsw'"
        )
        .fetch_one(store.pool())
        .await
        {
            Ok(count) if count > 0 => "ok",
            Ok(_) => "degraded",
            Err(_) => "degraded",
        }
    } else {
        "down"
    };

    // Embeddings status: ok if ready (daemon loaded model), degraded otherwise
    let embeddings_status = if is_ready { "ok" } else { "degraded" };

    // Model name: derive from embedding config (local_model or openai_model depending on provider)
    let model_name = if state.config.embedding.provider == "openai" {
        state.config.embedding.openai_model.clone()
    } else {
        state.config.embedding.local_model.clone()
    };

    let overall = if db_status == "ok" && is_ready {
        "ok"
    } else if is_ready {
        "degraded"
    } else {
        "starting"
    };

    let resp = serde_json::json!({
        "status": overall,
        "components": {
            "db": {
                "status": db_status,
                "pool_active": pool_active,
                "pool_idle": pool_idle,
            },
            "embeddings": {
                "status": embeddings_status,
                "pending": pending_embeddings,
                "model": model_name,
            },
            "hnsw": {
                "status": hnsw_status,
            },
        },
        "resources": {
            "memories": {
                "current": memory_count,
                "limit": state.caps.max_memories,
            },
            "embedding_batch_size": {
                "current": serde_json::Value::Null,
                "limit": state.caps.max_embedding_batch_size as u64,
            },
            "search_results": {
                "current": serde_json::Value::Null,
                "limit": state.caps.max_search_results as u64,
            },
            "db_connections": {
                "current": db_conn_count,
                "limit": state.caps.max_db_connections as u64,
            },
        },
        "uptime_secs": uptime,
    });

    let code = if is_ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, Json(resp))
}

/// Start the health HTTP server.
///
/// Non-fatal: if bind fails, logs a warning and returns rather than crashing the daemon.
/// Orchestrators should wait for /health to return 200 before sending traffic.
pub async fn serve(addr: SocketAddr, state: AppState) {
    // Apply per-endpoint rate limits, then wrap with metrics middleware.
    // /health, /status, and /metrics are NOT in api_routes — they are never metered.
    let api_routes = crate::transport::api::router(&state.config.rate_limit)
        .layer(axum::middleware::from_fn(
            crate::transport::metrics::metrics_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .route("/metrics", get(crate::transport::metrics::metrics_handler))
        .merge(api_routes)
        .with_state(state)
        // TraceLayer is the outermost layer — every HTTP request gets a span with
        // request_id, method, and endpoint for structured log correlation.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<_>| {
                    let request_id = uuid::Uuid::new_v4().to_string();
                    tracing::info_span!(
                        "http_request",
                        request_id = %request_id,
                        method = %req.method(),
                        endpoint = %req.uri().path(),
                    )
                }),
        );

    tracing::info!(%addr, "Health HTTP server starting");

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "Health HTTP server exited with error");
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                %addr,
                "Health HTTP server failed to bind — health endpoints unavailable (non-fatal)"
            );
        }
    }
}
