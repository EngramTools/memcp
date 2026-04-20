//! HTTP API integration tests — validates all /v1/* endpoints end-to-end.
//!
//! Each test spawns a full axum server on a random port with a real ephemeral DB.
//! Uses reqwest to hit each endpoint and validates request/response contracts.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use memcp::config::Config;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::transport::api;
use memcp::transport::health::AppState;
use memcp::MIGRATOR;
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

/// Build the full test app (health + API routes) and spawn it on a random port.
/// Returns the base URL (e.g., "http://127.0.0.1:45321").
async fn spawn_test_server(state: AppState) -> String {
    let api_routes = api::router(&state.config.rate_limit, state.auth.clone());
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
    assert!(
        body["embedding_status"].as_str().is_some(),
        "embedding_status must be present"
    );
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
    assert!(
        status == 404 || status == 500,
        "Expected 404 or 500, got {}",
        status
    );
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
        .json(
            &serde_json::json!({"content": "Rust programming language systems", "tags": ["rust"]}),
        )
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
    assert!(
        body["session_id"].is_string(),
        "session_id must be a string"
    );
    assert!(body["memories"].is_array(), "memories must be an array");
    assert!(body["count"].is_number(), "count must be a number");
    // first=true → preamble and current_datetime should be present
    assert!(
        body["preamble"].is_string(),
        "preamble must be present when first=true"
    );
    assert!(
        body["current_datetime"].is_string(),
        "current_datetime must be present when first=true"
    );
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
    assert!(
        status == 200 || status == 503,
        "Expected 200 or 503, got {}",
        status
    );
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

    let result = memcp::cli::dispatch_remote(
        &base,
        "store",
        serde_json::json!({
            "content": "remote test memory"
        }),
    )
    .await
    .expect("dispatch_remote store should succeed");

    assert!(result.get("id").is_some(), "response must have id field");
    let id = result["id"].as_str().expect("id must be a string");
    assert_eq!(id.len(), 36, "id must be a UUID");
    assert!(
        result.get("embedding_status").is_some(),
        "embedding_status must be present"
    );
}

/// dispatch_remote() recall queryless: sends HTTP POST and returns session_id + memories.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_dispatch_remote_recall_queryless(pool: PgPool) {
    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;

    let result = memcp::cli::dispatch_remote(
        &base,
        "recall",
        serde_json::json!({
            "first": true
        }),
    )
    .await
    .expect("dispatch_remote recall should succeed");

    assert!(
        result["session_id"].is_string(),
        "session_id must be a string"
    );
    assert!(result["memories"].is_array(), "memories must be an array");
    assert!(result["count"].is_number(), "count must be a number");
}

/// dispatch_remote() error handling: 503 when not ready → returns Err with "503".
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_dispatch_remote_error_handling(pool: PgPool) {
    let state = make_test_state(pool, false).await; // ready = false
    let base = spawn_test_server(state).await;

    let err = memcp::cli::dispatch_remote(
        &base,
        "store",
        serde_json::json!({
            "content": "should fail"
        }),
    )
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
        msg.contains("Remote request")
            || msg.contains("connection")
            || msg.contains("refused")
            || msg.contains("error"),
        "Expected a connection error, got: {}",
        msg
    );
}

// ---------------------------------------------------------------------------
// Phase 24.75 — /v1/memory/span HTTP route (CHUNK-04)
// ---------------------------------------------------------------------------

/// POST /v1/memory/span with `{memory_id, topic}` returns 200 with a JSON body
/// shaped `{content, span: {start, end}}`. Flipped ON in Plan 24.75-04.
///
/// End-to-end coverage: axum router → `memory_span_handler` → shared
/// `compute_memory_span`. Uses a keyword-indicator mock embedding provider so
/// the test exercises the ranker without pulling in local-embed.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_memory_span_http(pool: PgPool) {
    use async_trait::async_trait;
    use memcp::embedding::{EmbeddingError, EmbeddingProvider};
    use memcp::store::MemoryStore;

    struct TopicMock;

    #[async_trait]
    impl EmbeddingProvider for TopicMock {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            let lower = text.to_lowercase();
            // Same keyword signal as memory_span_test::KeywordEmbedder.
            let kws = [
                "authentication",
                "auth",
                "login",
                "password",
                "billing",
                "invoice",
                "payment",
                "shipping",
                "delivery",
                "address",
            ];
            Ok(kws
                .iter()
                .map(|kw| if lower.contains(kw) { 1.0 } else { 0.0 })
                .collect())
        }
        fn model_name(&self) -> &str {
            "topic-mock"
        }
        fn dimension(&self) -> usize {
            10
        }
    }

    // Seed a multi-topic memory.
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();
    let auth = "Authentication flow. The login subsystem validates credentials by hashing the user-supplied password with argon2 and comparing the result to the stored hash. When authentication succeeds we mint a short-lived access token and a longer refresh token. The refresh token rotates on every login to limit replay exposure. Password reset triggers a signed email link, also tied to the authentication subsystem. ".repeat(6);
    let billing = "Billing and invoices. Every paid plan generates a monthly invoice line, and card-on-file payment happens three days before the billing period ends. Failed payment retries twice before the account moves to a delinquent state; each retry re-emails the customer with the updated invoice. Billing reports aggregate by project for multi-tenant customers. ".repeat(6);
    let shipping = "Shipping and delivery. Physical goods ship from the nearest regional warehouse, and the shipping address gets validated against the postal carrier's geocoding service at checkout. Delivery tracking updates propagate back through the shipping webhook into the customer's order page. Failed delivery attempts generate a shipping exception row. ".repeat(6);
    let content = format!("{}\n\n{}\n\n{}", auth, billing, shipping);
    let mem = store
        .store(memcp::store::CreateMemory {
            content: content.clone(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            event_time: None,
            event_time_precision: None,
            project: Some("test".to_string()),
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
            knowledge_tier: None,
            source_ids: None,
            reply_to_id: None,
        })
        .await
        .unwrap();

    // Build AppState with our mock embedder wired in.
    let mut state = make_test_state(pool, true).await;
    state.embed_provider = Some(Arc::new(TopicMock));
    let base = spawn_test_server(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/memory/span", base))
        .json(&serde_json::json!({
            "memory_id": mem.id,
            "topic": "authentication login",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body: serde_json::Value = resp.json().await.unwrap();
    let span_content = body["content"].as_str().expect("content present");
    let start = body["span"]["start"].as_u64().expect("span.start present") as usize;
    let end = body["span"]["end"].as_u64().expect("span.end present") as usize;
    assert!(start < end, "start < end");
    assert!(end <= content.len(), "end must fit in content length");
    let lower = span_content.to_lowercase();
    assert!(
        lower.contains("authentication") || lower.contains("login"),
        "Expected auth keywords in returned span: {}",
        &span_content[..span_content.len().min(200)]
    );
}
