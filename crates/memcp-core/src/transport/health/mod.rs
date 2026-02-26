/// Health HTTP server for container lifecycle probes.
///
/// Provides:
///   GET /health — liveness/readiness probe (200 = ready, 503 = starting/not ready)
///   GET /status — operational status with component health and resource usage vs limits
///
/// Runs on a separate configurable port (default: 9090) from the MCP stdio transport.
/// Bind failure is non-fatal: logs a warning and returns rather than crashing the daemon.

use axum::{Router, Json, extract::State, routing::get, http::StatusCode};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::net::SocketAddr;
use tokio::time::Instant;

use crate::config::ResourceCapsConfig;

/// Shared state for the health server.
#[derive(Clone)]
pub struct HealthState {
    /// Set to true after DB connect + migrations complete.
    pub ready: Arc<AtomicBool>,
    /// Startup instant for uptime calculation.
    pub started_at: Instant,
    /// Resource caps from config.
    pub caps: ResourceCapsConfig,
    /// Postgres store for live queries (/status).
    pub store: Option<Arc<crate::store::postgres::PostgresMemoryStore>>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    components: ComponentHealth,
    resources: ResourceUsage,
    uptime_secs: u64,
}

#[derive(Serialize)]
struct ComponentHealth {
    db: &'static str,
    embeddings: &'static str,
    hnsw: &'static str,
}

#[derive(Serialize)]
struct ResourceInfo {
    current: Option<u64>,
    limit: Option<u64>,
}

#[derive(Serialize)]
struct ResourceUsage {
    memories: ResourceInfo,
    embedding_batch_size: ResourceInfo,
    search_results: ResourceInfo,
    db_connections: ResourceInfo,
}

/// GET /health — liveness/readiness probe.
/// Returns 200 when ready, 503 when not ready.
async fn health_handler(State(state): State<HealthState>) -> (StatusCode, Json<HealthResponse>) {
    if state.ready.load(Ordering::Acquire) {
        (StatusCode::OK, Json(HealthResponse { status: "ok" }))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(HealthResponse { status: "starting" }))
    }
}

/// GET /status — operational status with component health and resource usage.
async fn status_handler(State(state): State<HealthState>) -> (StatusCode, Json<StatusResponse>) {
    let is_ready = state.ready.load(Ordering::Acquire);
    let uptime = state.started_at.elapsed().as_secs();

    // Check DB connectivity and gather resource counts
    let (db_status, memory_count, db_conn_count) = if let Some(ref store) = state.store {
        // Quick connectivity check via pool
        let pool = store.pool();
        let pool_size = pool.size() as u64;

        match store.count_live_memories().await {
            Ok(count) => ("ok", Some(count as u64), Some(pool_size)),
            Err(_) => ("degraded", None, Some(pool_size)),
        }
    } else {
        ("down", None, None)
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

    let overall = if db_status == "ok" && is_ready {
        "ok"
    } else if is_ready {
        "degraded"
    } else {
        "starting"
    };

    let resp = StatusResponse {
        status: overall,
        components: ComponentHealth {
            db: db_status,
            embeddings: embeddings_status,
            hnsw: hnsw_status,
        },
        resources: ResourceUsage {
            memories: ResourceInfo {
                current: memory_count,
                limit: state.caps.max_memories,
            },
            embedding_batch_size: ResourceInfo {
                current: None, // config limit, not a live count
                limit: Some(state.caps.max_embedding_batch_size as u64),
            },
            search_results: ResourceInfo {
                current: None, // config limit, not a live count
                limit: Some(state.caps.max_search_results as u64),
            },
            db_connections: ResourceInfo {
                current: db_conn_count,
                limit: Some(state.caps.max_db_connections as u64),
            },
        },
        uptime_secs: uptime,
    };

    let code = if is_ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, Json(resp))
}

/// Start the health HTTP server.
///
/// Non-fatal: if bind fails, logs a warning and returns rather than crashing the daemon.
/// Orchestrators should wait for /health to return 200 before sending traffic.
pub async fn serve(addr: SocketAddr, state: HealthState) {
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/status", get(status_handler))
        .with_state(state);

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
