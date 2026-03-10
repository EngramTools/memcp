//! Rate limiting integration tests — validates the GovernorLayer produces correct 429
//! responses with Retry-After headers and JSON bodies, and that disabled mode allows
//! all requests through.
//!
//! Strategy: configure extremely low limits (rps=1, burst=1) so a burst of rapid requests
//! guarantees a 429 without requiring real-time measurement.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::OnceLock;

use axum::routing::get;
use axum::Router;
use memcp::config::{Config, RateLimitConfig};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::transport::api;
use memcp::transport::health::AppState;
use memcp::MIGRATOR;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Shared recorder — rate_limit tests may run in the same process as metrics_test.
// We must never install a second global recorder (would panic).
// ---------------------------------------------------------------------------

static RECORDER: OnceLock<PrometheusHandle> = OnceLock::new();

fn get_or_install_recorder() -> PrometheusHandle {
    RECORDER
        .get_or_init(|| {
            // Try to install; if already installed (by metrics_test running in same process),
            // fall back to a non-global recorder that won't panic.
            match std::panic::catch_unwind(|| {
                memcp::transport::metrics::install_prometheus_recorder()
            }) {
                Ok(h) => h,
                Err(_) => {
                    // Recorder already installed globally — build a local one for the handle.
                    metrics_exporter_prometheus::PrometheusBuilder::new()
                        .build_recorder()
                        .handle()
                }
            }
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build AppState with rate limiting enabled at very low limits for testing.
async fn make_rate_limited_state(pool: PgPool, store_rps: u32, burst_multiplier: u32) -> AppState {
    let handle = get_or_install_recorder();
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let mut config = Config::default();
    // Enable rate limiting with very low store limits to trigger 429 reliably
    config.rate_limit = RateLimitConfig {
        enabled: true,
        store_rps,
        burst_multiplier,
        // Keep other limits high so they don't interfere
        global_rps: 1000,
        recall_rps: 1000,
        search_rps: 1000,
        annotate_rps: 1000,
        update_rps: 1000,
        discover_rps: 1000,
        delete_rps: 1000,
        export_rps: 1000,
    };
    AppState {
        ready: Arc::new(AtomicBool::new(true)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle: handle,
        redaction_engine: None,
    }
}

/// Build AppState with rate limiting disabled.
async fn make_unlimited_state(pool: PgPool) -> AppState {
    let handle = get_or_install_recorder();
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let mut config = Config::default();
    config.rate_limit.enabled = false;
    AppState {
        ready: Arc::new(AtomicBool::new(true)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle: handle,
        redaction_engine: None,
    }
}

/// Spawn the app on a random port. Returns base URL.
async fn spawn_test_server(state: AppState) -> String {
    let api_routes = api::router(&state.config.rate_limit);

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

/// Send N rapid /v1/store requests concurrently. Returns all status codes.
async fn burst_store(base: &str, count: usize) -> Vec<u16> {
    let client = reqwest::Client::new();
    let mut handles = Vec::with_capacity(count);

    for _ in 0..count {
        let client = client.clone();
        let url = format!("{}/v1/store", base);
        handles.push(tokio::spawn(async move {
            client
                .post(&url)
                .json(&serde_json::json!({"content": "rate limit test memory"}))
                .send()
                .await
                .map(|r| r.status().as_u16())
                .unwrap_or(0)
        }));
    }

    let mut statuses = Vec::with_capacity(count);
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }
    statuses
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// With store_rps=1 burst=1, sending many rapid concurrent requests must produce at least
/// one HTTP 429.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_rate_limit_returns_429(pool: PgPool) {
    // rps=1, burst=1 → exactly 1 request allowed before refill
    let state = make_rate_limited_state(pool, 1, 1).await;
    let base = spawn_test_server(state).await;

    // Send burst of concurrent requests — token bucket has size 1, so extras get 429
    let statuses = burst_store(&base, 10).await;

    let has_429 = statuses.iter().any(|&s| s == 429);
    assert!(
        has_429,
        "Expected at least one 429 in responses from burst: {:?}",
        statuses
    );
}

/// On a 429 response, the `Retry-After` header must be present.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_rate_limit_429_has_retry_after_header(pool: PgPool) {
    let state = make_rate_limited_state(pool, 1, 1).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Sequential: first request takes the token, second must be 429
    // Send several to guarantee we get a 429
    let mut rate_limited_resp = None;
    for _ in 0..5 {
        let resp = client
            .post(format!("{}/v1/store", base))
            .json(&serde_json::json!({"content": "header test"}))
            .send()
            .await
            .unwrap();
        if resp.status().as_u16() == 429 {
            rate_limited_resp = Some(resp);
            break;
        }
    }

    let resp = rate_limited_resp.expect("Expected at least one 429 response in burst");
    assert!(
        resp.headers().contains_key("retry-after"),
        "Expected Retry-After header on 429 response, headers: {:?}",
        resp.headers()
    );
}

/// On a 429 response, the JSON body must contain `"error": "rate limited"` and
/// `"retry_after_ms"` as a number.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_rate_limit_429_has_json_body(pool: PgPool) {
    let state = make_rate_limited_state(pool, 1, 1).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Find a 429 response to inspect its body
    let mut rate_limited_resp = None;
    for _ in 0..5 {
        let resp = client
            .post(format!("{}/v1/store", base))
            .json(&serde_json::json!({"content": "body test"}))
            .send()
            .await
            .unwrap();
        if resp.status().as_u16() == 429 {
            rate_limited_resp = Some(resp);
            break;
        }
    }

    let resp = rate_limited_resp.expect("Expected at least one 429 response");

    // Verify content-type is JSON
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("application/json"),
        "Expected Content-Type: application/json on 429, got: {}",
        content_type
    );

    let body: serde_json::Value = resp.json().await.unwrap();

    // Must have "error": "rate limited"
    assert_eq!(
        body["error"].as_str(),
        Some("rate limited"),
        "Expected error='rate limited' in 429 body, got: {}",
        body
    );

    // Must have "retry_after_ms" as a number
    assert!(
        body["retry_after_ms"].is_number(),
        "Expected retry_after_ms to be a number in 429 body, got: {}",
        body
    );
}

/// With rate_limit.enabled=false, many rapid requests should all be non-429.
/// Verifies the disabled path in api::router() bypasses GovernorLayer entirely.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_rate_limit_disabled_allows_all(pool: PgPool) {
    let state = make_unlimited_state(pool).await;
    let base = spawn_test_server(state).await;

    // Send 10 concurrent requests — none should be 429 when rate limiting is disabled
    let statuses = burst_store(&base, 10).await;

    for status in &statuses {
        assert_ne!(
            *status, 429,
            "Expected no 429 when rate limiting is disabled, statuses: {:?}",
            statuses
        );
    }
}

/// GET /health is not inside /v1/* so it should never be rate-limited.
/// With extremely low limits, many /health requests must still all succeed.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_rate_limit_health_not_limited(pool: PgPool) {
    // Very low store limits that would 429 /v1/store immediately
    let state = make_rate_limited_state(pool, 1, 1).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Send many /health requests concurrently — they should never return 429
    let mut handles = Vec::new();
    for _ in 0..20 {
        let client = client.clone();
        let url = format!("{}/health", base);
        handles.push(tokio::spawn(async move {
            client
                .get(&url)
                .send()
                .await
                .map(|r| r.status().as_u16())
                .unwrap_or(0)
        }));
    }

    let statuses: Vec<u16> = {
        let mut s = Vec::new();
        for h in handles {
            s.push(h.await.unwrap());
        }
        s
    };

    for status in &statuses {
        assert_ne!(
            *status, 429,
            "Expected /health to never return 429, statuses: {:?}",
            statuses
        );
    }
}
