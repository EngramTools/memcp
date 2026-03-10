//! Prometheus metrics infrastructure.
//!
//! Installs the global metrics recorder, describes all metrics, and provides
//! the /metrics endpoint handler and pool metrics background poller.

use metrics::{
    counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram, Unit,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// Install the Prometheus recorder as the global metrics backend.
/// Returns a handle used by the /metrics endpoint to render scrape text.
/// MUST be called exactly once at daemon startup, before any metrics macros.
pub fn install_prometheus_recorder() -> PrometheusHandle {
    let duration_buckets = &[
        0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Suffix("duration_seconds".to_string()),
            duration_buckets,
        )
        .expect("valid bucket config")
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}

/// Register metric descriptions. Call once at startup after recorder install.
pub fn describe_metrics() {
    describe_counter!(
        "memcp_requests_total",
        "Total HTTP API requests by endpoint and status"
    );
    describe_histogram!(
        "memcp_request_duration_seconds",
        Unit::Seconds,
        "HTTP API request duration by endpoint"
    );
    describe_gauge!(
        "memcp_memories_total",
        "Total live (non-deleted) memories in store"
    );
    describe_gauge!(
        "memcp_memories_pending_embedding",
        "Memories with embedding_status=pending"
    );
    describe_counter!(
        "memcp_embedding_jobs_total",
        "Embedding jobs processed by status (success/error)"
    );
    describe_histogram!(
        "memcp_embedding_duration_seconds",
        Unit::Seconds,
        "Embedding job processing duration by tier"
    );
    describe_histogram!(
        "memcp_recall_memories_returned",
        "Number of memories returned per recall call"
    );
    describe_histogram!(
        "memcp_search_results_returned",
        "Number of results returned per search call"
    );
    describe_gauge!(
        "memcp_db_pool_connections",
        "DB connection pool connections by state (active/idle)"
    );
    describe_histogram!(
        "memcp_db_pool_acquire_duration_seconds",
        Unit::Seconds,
        "Time to acquire a DB connection from pool"
    );
    describe_counter!("memcp_gc_runs_total", "Total GC worker runs");
    describe_counter!("memcp_gc_pruned_total", "Total memories pruned by GC");
    describe_counter!(
        "memcp_dedup_merges_total",
        "Total deduplication merges performed"
    );
    describe_counter!(
        "memcp_enrichment_sweeps_total",
        "Enrichment worker sweep runs"
    );
    describe_counter!(
        "memcp_enrichment_memories_total",
        "Memories enriched with neighbor tags"
    );
    describe_counter!(
        "memcp_promotion_sweeps_total",
        "Promotion sweep worker runs"
    );
    describe_counter!(
        "memcp_promotion_promoted_total",
        "Memories promoted to quality embedding tier"
    );
    describe_counter!("memcp_curation_runs_total", "Curation worker pass runs");
    describe_counter!("memcp_curation_merged_total", "Memories merged by curation");
    describe_counter!(
        "memcp_curation_flagged_total",
        "Memories flagged stale by curation"
    );
    describe_counter!(
        "memcp_temporal_extractions_total",
        "Temporal event-time LLM extractions completed"
    );
    describe_histogram!(
        "memcp_discover_results_returned",
        "Results returned per discover call"
    );
}

/// Tower middleware recording request count and duration for /v1/* routes.
///
/// Applied only to the API sub-router (not /health, /status, or /metrics).
/// Uses global metrics macros — no AppState dependency.
pub async fn metrics_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let endpoint = req.uri().path().to_string();
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    let duration = start.elapsed().as_secs_f64();
    counter!("memcp_requests_total", "endpoint" => endpoint.clone(), "status" => status)
        .increment(1);
    histogram!("memcp_request_duration_seconds", "endpoint" => endpoint).record(duration);
    response
}

/// Spawn a background task that polls pool stats every `interval` and writes Prometheus gauges.
pub fn spawn_pool_metrics_poller(pool: Arc<PgPool>, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let total = pool.size() as f64;
            let idle = pool.num_idle() as f64;
            let active = total - idle;
            gauge!("memcp_db_pool_connections", "state" => "active").set(active);
            gauge!("memcp_db_pool_connections", "state" => "idle").set(idle);
        }
    });
}

/// GET /metrics handler — returns Prometheus scrape text.
pub async fn metrics_handler(
    axum::extract::State(state): axum::extract::State<super::health::AppState>,
) -> String {
    state.metrics_handle.render()
}
