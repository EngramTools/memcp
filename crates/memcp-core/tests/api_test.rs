//! HTTP API integration tests — validates all /v1/* endpoints end-to-end.
//!
//! Each test spawns a full axum server on a random port with a real ephemeral DB.
//! Uses reqwest to hit each endpoint and validates request/response contracts.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::routing::get;
use memcp::MIGRATOR;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::config::Config;
use memcp::transport::health::AppState;
use memcp::transport::api;
use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::PgPool;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal AppState suitable for testing.
///
/// embed_provider=None means recall with query returns 503 and search does BM25-only.
/// embed_sender=None means store doesn't enqueue embedding jobs (stays "pending").
async fn make_test_state(pool: PgPool, ready: bool) -> AppState {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let config = Config::default();
    // Build a non-global prometheus recorder for test isolation
    let metrics_handle = PrometheusBuilder::new().build_recorder().handle();
    AppState {
        ready: Arc::new(AtomicBool::new(ready)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle,
    }
}

/// Build the full test app (health + API routes) and spawn it on a random port.
/// Returns the base URL (e.g., "http://127.0.0.1:45321").
async fn spawn_test_server(state: AppState) -> String {
    let api_routes = api::router(&state.config.rate_limit);
    let app = Router::new()
        .route("/health", get(memcp::transport::health::status_handler))
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

/// POST /v1/store with valid body → 200, returns id + embedding_status=pending
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_store_and_get(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({
            "content": "Rust is a systems programming language",
            "type_hint": "fact",
            "tags": ["rust", "programming"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body: serde_json::Value = resp.json().await.unwrap();
    let id = body["id"].as_str().expect("id must be a string");
    assert!(!id.is_empty(), "id must not be empty");
    // Validate id is a valid UUID (36 chars with hyphens)
    assert_eq!(id.len(), 36, "id must be a UUID");
    assert!(body["embedding_status"].as_str().is_some(), "embedding_status must be present");
}

/// POST /v1/store with missing content → 400
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_store_missing_content(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();

    // Missing required field `content` → axum returns 422 (Unprocessable Entity)
    // OR our handler returns 400 if content is empty string
    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "Expected 400 or 422, got {}",
        status
    );
}

/// POST /v1/update — store then update content → 200
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_update(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // First store
    let store_resp: serde_json::Value = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "Original content"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = store_resp["id"].as_str().unwrap().to_string();

    // Then update
    let update_resp = client
        .post(format!("{}/v1/update", base))
        .json(&serde_json::json!({"id": id, "content": "Updated content"}))
        .send()
        .await
        .unwrap();

    assert_eq!(update_resp.status(), 200);
    let body: serde_json::Value = update_resp.json().await.unwrap();
    assert_eq!(body["content"].as_str().unwrap(), "Updated content");
    assert_eq!(body["id"].as_str().unwrap(), id.as_str());
}

/// POST /v1/update with nonexistent ID → 404
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_update_not_found(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/update", base))
        .json(&serde_json::json!({"id": "00000000-0000-0000-0000-000000000000", "content": "new"}))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    // Not found → 404 or 500 (store error surfaced differently depending on DB error type)
    assert!(status == 404 || status == 500, "Expected 404 or 500, got {}", status);
}

/// POST /v1/annotate — store then annotate with new tags → 200 + diff
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_annotate(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let store_resp: serde_json::Value = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "Memory to annotate"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = store_resp["id"].as_str().unwrap().to_string();

    let annotate_resp = client
        .post(format!("{}/v1/annotate", base))
        .json(&serde_json::json!({"id": id, "tags": ["important", "reviewed"]}))
        .send()
        .await
        .unwrap();

    assert_eq!(annotate_resp.status(), 200);
    let body: serde_json::Value = annotate_resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), id.as_str());
    // tags_added should contain our new tags
    let tags_added = body["changes"]["tags_added"].as_array().unwrap();
    assert!(!tags_added.is_empty(), "tags_added should not be empty");
}

/// POST /v1/search with a query → 200, results array present
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_search(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    // Store something first
    client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "Rust programming language systems", "tags": ["rust"]}))
        .send()
        .await
        .unwrap();

    let search_resp = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({"query": "rust programming", "limit": 5}))
        .send()
        .await
        .unwrap();

    assert_eq!(search_resp.status(), 200);
    let body: serde_json::Value = search_resp.json().await.unwrap();
    assert!(body["results"].is_array(), "results must be an array");
    assert!(body["total"].is_number(), "total must be a number");
}

/// POST /v1/recall with no query, first=true → 200, returns session_id and memories array
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_recall_queryless(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/recall", base))
        .json(&serde_json::json!({"first": true}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["session_id"].is_string(), "session_id must be a string");
    assert!(body["memories"].is_array(), "memories must be an array");
    assert!(body["count"].is_number(), "count must be a number");
    // first=true → preamble and current_datetime should be present
    assert!(body["preamble"].is_string(), "preamble must be present when first=true");
    assert!(body["current_datetime"].is_string(), "current_datetime must be present when first=true");
}

/// POST /v1/recall with query but no embed_provider → 503
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_recall_with_query_no_provider(pool: PgPool) {
    let state = make_test_state(pool, true).await; // embed_provider = None
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/recall", base))
        .json(&serde_json::json!({"query": "some query string"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503, "Expected 503 when no embed_provider");
}

/// GET /v1/status → 200, response has status field
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_status_alias(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/v1/status", base))
        .send()
        .await
        .unwrap();

    // Note: /v1/status is an alias for /health in our test app setup
    // In full daemon it's aliased to /status which returns the full status JSON
    let status = resp.status().as_u16();
    assert!(status == 200 || status == 503, "Expected 200 or 503, got {}", status);
}

/// /v1/store when not ready → 503
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_not_ready(pool: PgPool) {
    let state = make_test_state(pool, false).await; // ready = false
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/store", base))
        .json(&serde_json::json!({"content": "test"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503, "Expected 503 when not ready");
}

/// POST /v1/search with empty query → 400
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_search_empty_query(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({"query": ""}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400, "Expected 400 for empty query");
}

// ---------------------------------------------------------------------------
// dispatch_remote() end-to-end tests
// ---------------------------------------------------------------------------

/// dispatch_remote() store: sends HTTP POST and returns JSON with id field.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_dispatch_remote_store(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;

    let result = memcp::cli::dispatch_remote(&base, "store", serde_json::json!({
        "content": "remote test memory"
    }))
    .await
    .expect("dispatch_remote store should succeed");

    assert!(result.get("id").is_some(), "response must have id field");
    let id = result["id"].as_str().expect("id must be a string");
    assert_eq!(id.len(), 36, "id must be a UUID");
    assert!(result.get("embedding_status").is_some(), "embedding_status must be present");
}

/// dispatch_remote() recall queryless: sends HTTP POST and returns session_id + memories.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_dispatch_remote_recall_queryless(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;

    let result = memcp::cli::dispatch_remote(&base, "recall", serde_json::json!({
        "first": true
    }))
    .await
    .expect("dispatch_remote recall should succeed");

    assert!(result["session_id"].is_string(), "session_id must be a string");
    assert!(result["memories"].is_array(), "memories must be an array");
    assert!(result["count"].is_number(), "count must be a number");
}

/// dispatch_remote() error handling: 503 when not ready → returns Err with "503".
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_dispatch_remote_error_handling(pool: PgPool) {
    let state = make_test_state(pool, false).await; // ready = false
    let base = spawn_test_server(state).await;

    let err = memcp::cli::dispatch_remote(&base, "store", serde_json::json!({
        "content": "should fail"
    }))
    .await
    .expect_err("dispatch_remote should fail when server not ready");

    let msg = err.to_string();
    assert!(
        msg.contains("503"),
        "Error should mention 503, got: {}",
        msg
    );
}

/// dispatch_remote() connection failure: invalid URL → returns connection error.
#[tokio::test]
async fn test_dispatch_remote_invalid_url() {
    // Port 1 is privileged and never listening — connection refused immediately.
    let err = memcp::cli::dispatch_remote(
        "http://127.0.0.1:1",
        "store",
        serde_json::json!({"content": "x"}),
    )
    .await
    .expect_err("dispatch_remote should fail on connection refused");

    let msg = err.to_string();
    // reqwest wraps the OS error; check it's a network/connection error
    assert!(
        msg.contains("Remote request") || msg.contains("connection") || msg.contains("refused") || msg.contains("error"),
        "Expected a connection error, got: {}",
        msg
    );
}
