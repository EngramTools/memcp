//! Prometheus metrics integration tests — validates /metrics endpoint, metrics middleware,
//! and pool status fields produced by the Phase 10 observability infrastructure.
//!
//! Tests spawn a full axum server on a random port with a real ephemeral DB.
//! The global Prometheus recorder is installed once via OnceLock (installing it multiple
//! times in the same process would panic). All tests share the same recorder — safe
//! because Prometheus counters are additive and tests only assert presence, not exact values.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::OnceLock;

use axum::routing::get;
use axum::Router;
use memcp::config::Config;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::transport::api;
use memcp::transport::health::AppState;
use memcp::MIGRATOR;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Recorder — installed exactly once per process
// ---------------------------------------------------------------------------

static RECORDER: OnceLock<PrometheusHandle> = OnceLock::new();

fn get_or_install_recorder() -> PrometheusHandle {
    RECORDER
        .get_or_init(|| memcp::transport::metrics::install_prometheus_recorder())
        .clone()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal AppState for metrics tests.
/// Uses the shared global recorder so /metrics output reflects all metric writes.
async fn make_test_state(pool: PgPool, ready: bool) -> AppState {
    let handle = get_or_install_recorder();
    memcp::transport::metrics::describe_metrics();
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let config = Config::default();
    AppState {
        ready: Arc::new(AtomicBool::new(ready)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle: handle,
        redaction_engine: None,
        auth: memcp::transport::api::auth::AuthState::default(),
        content_filter: None,
        summarization_provider: None,
        extract_sender: None,
        topic_embedding_cache: Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
    }
}

/// Spawn the full app (health + /metrics + /v1/* routes with metrics middleware) on a random port.
/// Matches the production serve() layout from health/mod.rs.
async fn spawn_test_server(state: AppState) -> String {
    let api_routes = api::router(&state.config.rate_limit, state.auth.clone()).layer(
        axum::middleware::from_fn(memcp::transport::metrics::metrics_middleware),
    );

    let app = Router::new()
        .route("/health", get(memcp::transport::health::status_handler))
        .route("/status", get(memcp::transport::health::status_handler))
        .route("/metrics", get(memcp::transport::metrics::metrics_handler))
        .merge(api_routes)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// GET /metrics returns 200 with Prometheus text format containing # HELP and # TYPE headers
/// and at minimum the declared memcp metric names.
///
/// The Prometheus recorder renders `# HELP`/`# TYPE` lines only after metrics are first
/// observed. We make a /v1/store request first to initialize the counter, then verify
/// the /metrics endpoint exposes the expected metric names.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_metrics_endpoint_returns_prometheus_text(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // First, /metrics returns 200 (even if empty before any metrics are recorded)
    let initial_resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    assert_eq!(initial_resp.status(), 200, "Expected 200 OK from /metrics");

    // Make a /v1/store request to trigger metric recording
    client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "test for metrics init"}))
        .send()
        .await
        .unwrap();

    // Now /metrics should contain populated Prometheus exposition format
    let resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "Expected 200 OK from /metrics after recording"
    );
    let body = resp.text().await.unwrap();

    // Must be Prometheus exposition format with help/type annotations
    assert!(
        body.contains("# HELP"),
        "Expected # HELP in /metrics output, got:\n{}",
        body
    );
    assert!(
        body.contains("# TYPE"),
        "Expected # TYPE in /metrics output, got:\n{}",
        body
    );

    // Core metric names declared in describe_metrics() must appear
    assert!(
        body.contains("memcp_requests_total"),
        "Expected memcp_requests_total in /metrics output, got:\n{}",
        body
    );
    assert!(
        body.contains("memcp_request_duration_seconds"),
        "Expected memcp_request_duration_seconds in /metrics output, got:\n{}",
        body
    );
}

/// GET /metrics should not include an entry for endpoint="/metrics" in memcp_requests_total.
/// The /metrics route is not inside the /v1/* sub-router so it bypasses metrics_middleware.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_metrics_endpoint_not_metered(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Hit /metrics twice to confirm it never gets counted
    client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    let resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();

    // There should be no memcp_requests_total line with endpoint="/metrics"
    let has_metrics_metered = body
        .lines()
        .any(|l| l.contains("memcp_requests_total") && l.contains(r#"endpoint="/metrics""#));

    assert!(
        !has_metrics_metered,
        "Expected /metrics NOT to be metered by metrics_middleware, but found it in output:\n{}",
        body
    );
}

/// GET /health should not increment memcp_requests_total.
/// The /health route is not inside the /v1/* sub-router so it bypasses metrics_middleware.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_health_endpoint_not_metered(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Hit /health and then check /metrics for any health-related counter entry
    client.get(format!("{}/health", base)).send().await.unwrap();
    let metrics_resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    let body = metrics_resp.text().await.unwrap();

    let has_health_metered = body
        .lines()
        .any(|l| l.contains("memcp_requests_total") && l.contains(r#"endpoint="/health""#));

    assert!(
        !has_health_metered,
        "Expected /health NOT to be metered by metrics_middleware, but found it in output:\n{}",
        body
    );
}

/// POST /v1/store → then GET /metrics should contain memcp_requests_total with endpoint="/v1/store".
/// Verifies metrics_middleware correctly labels /v1/* endpoint paths.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_api_request_increments_counter(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Make a /v1/store request
    let store_resp = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({
            "content": "Rust is a systems programming language",
            "type_hint": "fact",
            "tags": ["rust"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(store_resp.status(), 200, "Store request must succeed");

    // Check /metrics for the counter
    let metrics_resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    let body = metrics_resp.text().await.unwrap();

    let has_store_counter = body
        .lines()
        .any(|l| l.contains("memcp_requests_total") && l.contains(r#"endpoint="/v1/store""#));

    assert!(
        has_store_counter,
        "Expected memcp_requests_total with endpoint=/v1/store in /metrics output:\n{}",
        body
    );
}

/// POST /v1/store → then GET /metrics should contain memcp_request_duration_seconds histogram.
/// Verifies duration histogram is emitted with bucket entries.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_api_request_records_duration(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Make a /v1/store request to trigger duration recording
    client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "duration test memory"}))
        .send()
        .await
        .unwrap();

    let metrics_resp = client
        .get(format!("{}/metrics", base))
        .send()
        .await
        .unwrap();
    let body = metrics_resp.text().await.unwrap();

    assert!(
        body.contains("memcp_request_duration_seconds"),
        "Expected memcp_request_duration_seconds histogram in /metrics output:\n{}",
        body
    );

    // Verify histogram has bucket entries (Prometheus format includes _bucket suffix)
    assert!(
        body.contains("memcp_request_duration_seconds_bucket"),
        "Expected histogram bucket entries in /metrics output"
    );
}

/// GET /status when ready with a real store should include pool breakdown fields.
/// Verifies Plan 02 enrichment: pool_active and pool_idle in components.db.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_status_shows_pool_details(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{}/status", base)).send().await.unwrap();

    assert_eq!(resp.status(), 200, "Expected 200 from /status when ready");

    let body: serde_json::Value = resp.json().await.unwrap();

    // Verify components.db exists
    let db_component = &body["components"]["db"];
    assert!(
        !db_component.is_null(),
        "Expected components.db in /status response"
    );

    // Verify pool breakdown fields are present (Plan 02 enrichment)
    assert!(
        !db_component["pool_active"].is_null(),
        "Expected pool_active field in components.db, got: {}",
        db_component
    );
    assert!(
        !db_component["pool_idle"].is_null(),
        "Expected pool_idle field in components.db, got: {}",
        db_component
    );

    // pool_active and pool_idle should be non-negative numbers
    let pool_active = db_component["pool_active"].as_u64();
    let pool_idle = db_component["pool_idle"].as_u64();
    assert!(pool_active.is_some(), "pool_active must be a number");
    assert!(pool_idle.is_some(), "pool_idle must be a number");
}
