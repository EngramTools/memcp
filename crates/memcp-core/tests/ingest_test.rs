//! Phase 24.5 /v1/ingest integration tests.
//!
//! Each test spawns a full axum server on a random port with a real ephemeral DB
//! and drives `POST /v1/ingest` through the shared per-message pipeline. Tests
//! that need a content filter or summarization provider wire a stub into AppState.
//!
//! RESEARCH Pitfall 6 note: the tool-count update to 18 lives in
//! `tests/integration_test.rs` and flips green only when MCP tool registration
//! lands in Plan 24.5-04.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use axum::routing::get;
use axum::Router;
use memcp::config::Config;
use memcp::content_filter::{ContentFilter, FilterVerdict};
use memcp::errors::MemcpError;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::summarization::{SummarizationError, SummarizationProvider};
use memcp::transport::api;
use memcp::transport::api::auth::AuthState;
use memcp::transport::health::AppState;
use memcp::MIGRATOR;
use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::PgPool;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

const SOURCE: &str = "telegram-bot";
const SESSION: &str = "sess-24.5-03-test";
const PROJECT: &str = "memcp";

/// ContentFilter stub that drops any content whose text contains the marker "SKIP".
struct SkipMarkerFilter;

#[async_trait]
impl ContentFilter for SkipMarkerFilter {
    async fn check(&self, content: &str) -> Result<FilterVerdict, MemcpError> {
        if content.contains("SKIP") {
            Ok(FilterVerdict::Drop {
                reason: "skip-marker".to_string(),
            })
        } else {
            Ok(FilterVerdict::Allow)
        }
    }
}

/// SummarizationProvider stub that returns a constant marker string.
struct StubSummarizer;

#[async_trait]
impl SummarizationProvider for StubSummarizer {
    async fn summarize(&self, _content: &str) -> Result<String, SummarizationError> {
        Ok("SUMMARIZED".to_string())
    }
    fn model_name(&self) -> &str {
        "stub"
    }
}

// ---------------------------------------------------------------------------
// State / harness helpers
// ---------------------------------------------------------------------------

struct IngestFixture {
    pub api_key: Option<String>,
    pub content_filter: Option<Arc<dyn ContentFilter>>,
    pub summarization: Option<Arc<dyn SummarizationProvider>>,
    pub enable_redaction: bool,
    /// When Some, overrides `config.ingest.max_batch_size`.
    pub max_batch: Option<usize>,
    pub enable_rate_limit: bool,
    pub ingest_rps: Option<u32>,
}

impl IngestFixture {
    fn default_open() -> Self {
        Self {
            api_key: None,
            content_filter: None,
            summarization: None,
            enable_redaction: false,
            max_batch: None,
            enable_rate_limit: false,
            ingest_rps: None,
        }
    }
}

async fn build_state(pool: PgPool, fixture: &IngestFixture) -> AppState {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let mut config = Config::default();
    if let Some(m) = fixture.max_batch {
        config.ingest.max_batch_size = m;
    }
    if fixture.enable_rate_limit {
        config.rate_limit.enabled = true;
        if let Some(rps) = fixture.ingest_rps {
            config.rate_limit.ingest_rps = rps;
        }
        // Keep other limits sky-high so cross-test flakes are impossible.
        config.rate_limit.store_rps = 10_000;
        config.rate_limit.recall_rps = 10_000;
        config.rate_limit.search_rps = 10_000;
        config.rate_limit.annotate_rps = 10_000;
        config.rate_limit.update_rps = 10_000;
        config.rate_limit.discover_rps = 10_000;
        config.rate_limit.delete_rps = 10_000;
        config.rate_limit.export_rps = 10_000;
        config.rate_limit.batch_get_rps = 10_000;
        config.rate_limit.global_rps = 10_000;
    } else {
        config.rate_limit.enabled = false;
    }

    let redaction_engine = if fixture.enable_redaction {
        config.redaction.secrets_enabled = true;
        memcp::pipeline::redaction::RedactionEngine::from_config(&config.redaction)
            .ok()
            .map(Arc::new)
    } else {
        None
    };

    let metrics_handle = PrometheusBuilder::new().build_recorder().handle();
    let auth = AuthState::from_optional(fixture.api_key.clone());
    AppState {
        ready: Arc::new(AtomicBool::new(true)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle,
        redaction_engine,
        auth,
        content_filter: fixture.content_filter.clone(),
        summarization_provider: fixture.summarization.clone(),
        extract_sender: None,
        topic_embedding_cache: Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        reasoning_creds: memcp::transport::health::ReasoningCreds::default(),
        reasoning_tenancy: memcp::transport::health::ReasoningTenancy::Byok,
    }
}

async fn spawn(state: AppState) -> String {
    let api_routes = api::router(
        &state.config.rate_limit,
        state.auth.clone(),
        state.reasoning_tenancy,
        state.reasoning_creds.clone(),
    );
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

fn msg(role: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "content": content,
    })
}

// ---------------------------------------------------------------------------
// INGEST-01 — HTTP transport + auth gate
// ---------------------------------------------------------------------------

/// INGEST-01: POST /v1/ingest returns 200 with a valid API key.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_basic(pool: PgPool) {
    let state = build_state(pool, &IngestFixture {
        api_key: Some("secret-key".to_string()),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "first"), msg("assistant", "second")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client
        .post(format!("{}/v1/ingest", base))
        .header("X-Memcp-Key", "secret-key")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "expected 200, got {}", resp.status());
    let rj: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(rj["summary"]["stored"], 2, "body: {rj}");
    assert_eq!(rj["results"].as_array().unwrap().len(), 2);
}

/// INGEST-01 / D-01: POST /v1/ingest returns 401 without key when a key is configured.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_auth_required(pool: PgPool) {
    let state = build_state(pool, &IngestFixture {
        api_key: Some("secret-key".to_string()),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "needs-auth")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client
        .post(format!("{}/v1/ingest", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "expected 401, got {}", resp.status());
}

/// INGEST-01 / D-02: POST /v1/ingest returns 200 without key when no key is configured (loopback mode).
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_loopback_no_auth(pool: PgPool) {
    let state = build_state(pool, &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "loopback-ok")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client
        .post(format!("{}/v1/ingest", base))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "expected 200, got {}", resp.status());
}

// ---------------------------------------------------------------------------
// INGEST-02 — Pipeline: redaction, tier, summarization, filter
// ---------------------------------------------------------------------------

/// INGEST-02 / D-10: Pipeline applies redaction before storage. Stored content is masked.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_redacts_secrets(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture {
        enable_redaction: true,
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    // Known secret-looking AWS access key — redaction rules ship this pattern.
    let raw = "My AWS key is AKIAIOSFODNN7EXAMPLE please don't log it";
    let body = serde_json::json!({
        "messages": [msg("user", raw)],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let rj: serde_json::Value = resp.json().await.unwrap();
    let memory_id = rj["results"][0]["memory_id"].as_str().expect("memory_id").to_string();

    let stored_content: String = sqlx::query_scalar("SELECT content FROM memories WHERE id = $1")
        .bind(&memory_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !stored_content.contains("AKIAIOSFODNN7EXAMPLE"),
        "secret must be redacted before store, got: {stored_content}"
    );
}

/// INGEST-02 / D-23: Ingested memory has knowledge_tier="raw", write_path="ingest",
/// trust_level ~= 0.3, session_id and project populated.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_tier_raw(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "tier check")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let rj: serde_json::Value = resp.json().await.unwrap();
    let memory_id = rj["results"][0]["memory_id"].as_str().unwrap().to_string();
    let row = sqlx::query_as::<_, (String, Option<String>, Option<f32>, Option<String>, Option<String>)>(
        "SELECT knowledge_tier, write_path, trust_level, session_id, project FROM memories WHERE id = $1",
    )
    .bind(&memory_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, "raw");
    assert_eq!(row.1.as_deref(), Some("ingest"));
    assert!(
        (row.2.unwrap_or(0.0) - 0.3).abs() < 0.01,
        "trust_level should be ~0.3, got {:?}",
        row.2
    );
    assert_eq!(row.3.as_deref(), Some(SESSION));
    assert_eq!(row.4.as_deref(), Some(PROJECT));
}

/// INGEST-02 / D-10+D-12: Assistant role triggers summarization; user role does not.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_summarizes_assistant(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture {
        summarization: Some(Arc::new(StubSummarizer)),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [
            msg("user", "raw user message"),
            msg("assistant", "long assistant response that should be summarized"),
        ],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let rj: serde_json::Value = resp.json().await.unwrap();
    let user_id = rj["results"][0]["memory_id"].as_str().unwrap().to_string();
    let assist_id = rj["results"][1]["memory_id"].as_str().unwrap().to_string();

    let (user_content, user_tags_json): (String, Option<serde_json::Value>) =
        sqlx::query_as("SELECT content, tags FROM memories WHERE id = $1")
            .bind(&user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (assist_content, assist_tags_json): (String, Option<serde_json::Value>) =
        sqlx::query_as("SELECT content, tags FROM memories WHERE id = $1")
            .bind(&assist_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(user_content, "raw user message", "user content unchanged");
    assert_eq!(assist_content, "SUMMARIZED", "assistant content summarized");

    let assist_tags: Vec<String> = assist_tags_json
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|x| x.to_string())).collect())
        .unwrap_or_default();
    assert!(
        assist_tags.iter().any(|t| t == "summarized"),
        "assistant memory should carry `summarized` tag, got: {assist_tags:?}"
    );

    let user_tags: Vec<String> = user_tags_json
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|x| x.to_string())).collect())
        .unwrap_or_default();
    assert!(
        !user_tags.iter().any(|t| t == "summarized"),
        "user memory must NOT carry `summarized` tag, got: {user_tags:?}"
    );
}

/// INGEST-02 / D-10: Content filter drops messages containing the marker string.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_filter_drops(pool: PgPool) {
    let state = build_state(pool, &IngestFixture {
        content_filter: Some(Arc::new(SkipMarkerFilter)),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "this message contains SKIP so drop it")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let rj: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(rj["summary"]["filtered"], 1);
    assert_eq!(rj["results"][0]["status"], "filtered");
    assert!(rj["results"][0]["reason"].as_str().unwrap().contains("skip"));
}

// ---------------------------------------------------------------------------
// INGEST-03 — Batch semantics
// ---------------------------------------------------------------------------

/// INGEST-03: Batch of N messages returns N results with indices 0..=N-1.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_batch_size(pool: PgPool) {
    let state = build_state(pool, &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let messages: Vec<_> = (0..5).map(|i| msg("user", &format!("msg-{i}"))).collect();
    let body = serde_json::json!({
        "messages": messages,
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let rj: serde_json::Value = resp.json().await.unwrap();
    let results = rj["results"].as_array().unwrap();
    assert_eq!(results.len(), 5);
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["index"], i);
        assert_eq!(r["status"], "stored");
    }
}

/// INGEST-03: Batch exceeding max_batch_size returns 400.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_batch_limit(pool: PgPool) {
    let state = build_state(pool, &IngestFixture {
        max_batch: Some(3),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let messages: Vec<_> = (0..4).map(|i| msg("user", &format!("msg-{i}"))).collect();
    let body = serde_json::json!({
        "messages": messages,
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let resp = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 400);
}

/// INGEST-03 / D-14: Duplicate re-post returns status="duplicate" with the original memory_id.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_duplicate_status(pool: PgPool) {
    let state = build_state(pool, &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "dup-content-x"), msg("assistant", "dup-content-y")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });

    let r1: serde_json::Value = client
        .post(format!("{}/v1/ingest", base))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id1_a = r1["results"][0]["memory_id"].as_str().unwrap().to_string();
    let id1_b = r1["results"][1]["memory_id"].as_str().unwrap().to_string();

    let r2: serde_json::Value = client
        .post(format!("{}/v1/ingest", base))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r2["results"][0]["status"], "duplicate");
    assert_eq!(r2["results"][1]["status"], "duplicate");
    assert_eq!(r2["results"][0]["memory_id"].as_str().unwrap(), id1_a);
    assert_eq!(r2["results"][1]["memory_id"].as_str().unwrap(), id1_b);
    assert_eq!(r2["summary"]["duplicate"], 2);
}

/// INGEST-03 / D-17: Within-batch auto-chain threads prev_id into each following message.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_within_batch_chain(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [
            msg("user", "chain-msg-0"),
            msg("assistant", "chain-msg-1"),
            msg("user", "chain-msg-2"),
        ],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let r: serde_json::Value = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap().json().await.unwrap();
    let id0 = r["results"][0]["memory_id"].as_str().unwrap().to_string();
    let id1 = r["results"][1]["memory_id"].as_str().unwrap().to_string();
    let id2 = r["results"][2]["memory_id"].as_str().unwrap().to_string();

    let row1: Option<String> = sqlx::query_scalar("SELECT reply_to_id FROM memories WHERE id = $1")
        .bind(&id1).fetch_one(&pool).await.unwrap();
    let row2: Option<String> = sqlx::query_scalar("SELECT reply_to_id FROM memories WHERE id = $1")
        .bind(&id2).fetch_one(&pool).await.unwrap();
    let row0: Option<String> = sqlx::query_scalar("SELECT reply_to_id FROM memories WHERE id = $1")
        .bind(&id0).fetch_one(&pool).await.unwrap();
    assert_eq!(row0, None, "first message has no predecessor");
    assert_eq!(row1.as_deref(), Some(id0.as_str()));
    assert_eq!(row2.as_deref(), Some(id1.as_str()));
}

/// INGEST-03 / D-18: Caller-supplied reply_to_id overrides the auto-chain for that message.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_caller_reply_to_override(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let override_id = "deadbeef-1111-2222-3333-444455556666";
    let body = serde_json::json!({
        "messages": [
            msg("user", "chain-override-0"),
            {
                "role": "assistant",
                "content": "chain-override-1",
                "reply_to_id": override_id
            },
        ],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });
    let r: serde_json::Value = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap().json().await.unwrap();
    let id1 = r["results"][1]["memory_id"].as_str().unwrap().to_string();
    let row1: Option<String> = sqlx::query_scalar("SELECT reply_to_id FROM memories WHERE id = $1")
        .bind(&id1).fetch_one(&pool).await.unwrap();
    assert_eq!(
        row1.as_deref(),
        Some(override_id),
        "caller override must win over auto-chain"
    );
}

// ---------------------------------------------------------------------------
// INGEST-04 — Provenance
// ---------------------------------------------------------------------------

/// INGEST-04: `source` field round-trips to the stored row.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_source_provenance(pool: PgPool) {
    let state = build_state(pool.clone(), &IngestFixture::default_open()).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "provenance-check")],
        "source": "custom-telegram-bot",
        "session_id": SESSION,
        "project": PROJECT,
    });
    let r: serde_json::Value = client.post(format!("{}/v1/ingest", base)).json(&body).send().await.unwrap().json().await.unwrap();
    let id = r["results"][0]["memory_id"].as_str().unwrap().to_string();
    let src: String = sqlx::query_scalar("SELECT source FROM memories WHERE id = $1")
        .bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(src, "custom-telegram-bot");
}

// ---------------------------------------------------------------------------
// INGEST-05 — Rate limiting
// ---------------------------------------------------------------------------

/// INGEST-05: Rate limit returns 429 on burst above configured capacity.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_ingest_rate_limit_burst(pool: PgPool) {
    let state = build_state(pool, &IngestFixture {
        enable_rate_limit: true,
        ingest_rps: Some(1),
        ..IngestFixture::default_open()
    }).await;
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "messages": [msg("user", "rate-limit-burst")],
        "source": SOURCE,
        "session_id": SESSION,
        "project": PROJECT,
    });

    let mut handles = Vec::new();
    for _ in 0..12 {
        let c = client.clone();
        let url = format!("{}/v1/ingest", base);
        let b = body.clone();
        handles.push(tokio::spawn(async move {
            c.post(&url).json(&b).send().await.map(|r| r.status().as_u16()).unwrap_or(0)
        }));
    }
    let mut codes = Vec::new();
    for h in handles {
        codes.push(h.await.unwrap());
    }
    assert!(
        codes.iter().any(|&c| c == 429),
        "expected at least one 429 in burst, got: {codes:?}"
    );
}

// ---------------------------------------------------------------------------
// INGEST-06 — MCP tools + CLI surface (Plan 24.5-04)
// ---------------------------------------------------------------------------

mod mcp_ingest_harness {
    //! Minimal stdio MCP client for Plan 24.5-04 ingest tool tests. Mirrors the
    //! more elaborate `McpTestClient` in `integration_test.rs` but kept local
    //! so this test binary doesn't depend on integration_test.rs internals.
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    pub struct McpChild {
        child: Child,
        tx: Sender<Value>,
        rx: Receiver<Value>,
    }

    impl McpChild {
        pub fn spawn_with_db_url(db_url: &str) -> Self {
            let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.pop();
            path.pop();
            path.push("target");
            path.push("debug");
            path.push("memcp");

            let mut cmd = Command::new(&path);
            cmd.arg("serve")
                .env("DATABASE_URL", db_url)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());

            let mut child = cmd.spawn().expect("spawn memcp serve");
            let mut stdin: ChildStdin = child.stdin.take().expect("stdin");
            let stdout = child.stdout.take().expect("stdout");

            let (req_tx, req_rx) = channel::<Value>();
            let (resp_tx, resp_rx) = channel::<Value>();

            thread::spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    let s = serde_json::to_string(&req).unwrap();
                    if writeln!(stdin, "{}", s).is_err() {
                        break;
                    }
                    if stdin.flush().is_err() {
                        break;
                    }
                }
            });

            thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match r.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                                if resp_tx.send(v).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                child,
                tx: req_tx,
                rx: resp_rx,
            }
        }

        pub fn send_request(&self, req: Value) -> Option<Value> {
            self.tx.send(req).ok()?;
            self.rx.recv_timeout(Duration::from_secs(15)).ok()
        }

        pub fn send_notification(&self, n: Value) {
            let _ = self.tx.send(n);
            thread::sleep(Duration::from_millis(50));
        }

        pub fn initialize(&self) {
            let init = json!({
                "jsonrpc": "2.0",
                "method": "initialize",
                "id": 1,
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "ingest-test", "version": "1.0"}
                }
            });
            self.send_request(init).expect("init");
            self.send_notification(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }));
        }
    }

    impl Drop for McpChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Provision an ephemeral temp DB, return (db_name, db_url).
    pub async fn provision_temp_db() -> (String, String) {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";
        let db_name = format!("memcp_test_{}", uuid::Uuid::new_v4().simple());
        let base_pool = sqlx::PgPool::connect(base_url).await.expect("connect base");
        sqlx::query(&format!("CREATE DATABASE {}", db_name))
            .execute(&base_pool)
            .await
            .expect("CREATE DATABASE");
        let test_url = format!("postgres://memcp:memcp@localhost:5433/{}", db_name);
        let test_pool = sqlx::PgPool::connect(&test_url).await.expect("connect temp");
        sqlx::migrate!("./migrations")
            .run(&test_pool)
            .await
            .expect("migrations");
        test_pool.close().await;
        base_pool.close().await;
        (db_name, test_url)
    }

    pub async fn drop_temp_db(db_name: &str) {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";
        let p = sqlx::PgPool::connect(base_url).await.expect("connect base");
        let _ = sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", db_name))
            .execute(&p)
            .await;
        p.close().await;
    }
}

/// INGEST-06 / D-22: MCP `ingest_messages` round-trips a batch.
#[tokio::test]
async fn test_mcp_ingest_messages() {
    use mcp_ingest_harness::*;
    let (db_name, db_url) = provision_temp_db().await;
    let client = McpChild::spawn_with_db_url(&db_url);
    // Give the child time to run migrations + warm up before initialize.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    client.initialize();

    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 42,
        "params": {
            "name": "ingest_messages",
            "arguments": {
                "messages": [
                    {"role": "user", "content": "hello from mcp batch"},
                    {"role": "assistant", "content": "reply from mcp batch"}
                ],
                "source": "mcp-test",
                "session_id": "mcp-batch-sess",
                "project": "memcp"
            }
        }
    });
    let resp = client.send_request(call).expect("mcp response");
    assert_eq!(resp["jsonrpc"], "2.0");
    let result = &resp["result"];
    assert!(
        result["isError"].is_null() || result["isError"] == false,
        "unexpected error: {result}"
    );
    let body = &result["structuredContent"];
    assert_eq!(body["summary"]["stored"], 2, "body: {body}");
    assert_eq!(body["results"].as_array().unwrap().len(), 2);

    drop(client);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    drop_temp_db(&db_name).await;
}

/// INGEST-06 / D-22: MCP `ingest_message` convenience tool (single-message wrapper).
#[tokio::test]
async fn test_mcp_ingest_message_single() {
    use mcp_ingest_harness::*;
    let (db_name, db_url) = provision_temp_db().await;
    let client = McpChild::spawn_with_db_url(&db_url);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    client.initialize();

    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 43,
        "params": {
            "name": "ingest_message",
            "arguments": {
                "role": "user",
                "content": "single mcp hello",
                "source": "mcp-test",
                "session_id": "mcp-single-sess",
                "project": "memcp"
            }
        }
    });
    let resp = client.send_request(call).expect("mcp response");
    let result = &resp["result"];
    assert!(
        result["isError"].is_null() || result["isError"] == false,
        "unexpected error: {result}"
    );
    let body = &result["structuredContent"];
    assert_eq!(body["summary"]["stored"], 1, "body: {body}");
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    assert_eq!(body["results"][0]["status"], "stored");

    drop(client);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    drop_temp_db(&db_name).await;
}

/// INGEST-06 / D-20: CLI `memcp ingest --file foo.jsonl` works.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_cli_ingest_file_jsonl(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();

    // JSONL fixture: two user/assistant turns on separate lines.
    let tmp = tempfile::NamedTempFile::with_suffix(".jsonl").unwrap();
    std::fs::write(
        tmp.path(),
        "{\"role\":\"user\",\"content\":\"jsonl first\"}\n\
         {\"role\":\"assistant\",\"content\":\"jsonl reply\"}\n",
    )
    .unwrap();
    let raw = std::fs::read_to_string(tmp.path()).unwrap();
    let value = memcp::cli::cmd_ingest_from_reader(
        &store,
        &config,
        std::io::Cursor::new(raw),
        "cli-test".to_string(),
        "cli-jsonl-sess".to_string(),
        "memcp".to_string(),
    )
    .await
    .expect("cmd_ingest_from_reader jsonl");
    assert_eq!(value["summary"]["stored"], 2, "body: {value}");
    assert_eq!(value["results"].as_array().unwrap().len(), 2);
}

/// INGEST-06 / D-20+21: CLI `memcp ingest --file foo.json` (JSON array) works.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_cli_ingest_file_array(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();

    // JSON-array fixture: single line, two objects in an array.
    let array = serde_json::json!([
        {"role": "user", "content": "array first"},
        {"role": "assistant", "content": "array reply"}
    ])
    .to_string();
    let value = memcp::cli::cmd_ingest_from_reader(
        &store,
        &config,
        std::io::Cursor::new(array),
        "cli-test".to_string(),
        "cli-array-sess".to_string(),
        "memcp".to_string(),
    )
    .await
    .expect("cmd_ingest_from_reader array");
    assert_eq!(value["summary"]["stored"], 2, "body: {value}");
    assert_eq!(value["results"].as_array().unwrap().len(), 2);
}

/// INGEST-06 / D-20+21: CLI `memcp ingest` from stdin auto-detects.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_cli_ingest_stdin(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();

    // Simulate stdin with a Cursor. `cmd_ingest_from_reader` is the exact seam
    // `cmd_ingest` uses when stdin is piped — so this covers the stdin path.
    let jsonl = "{\"role\":\"user\",\"content\":\"stdin first\"}\n\
                 {\"role\":\"assistant\",\"content\":\"stdin second\"}\n";
    let value = memcp::cli::cmd_ingest_from_reader(
        &store,
        &config,
        std::io::Cursor::new(jsonl),
        "cli-test".to_string(),
        "cli-stdin-sess".to_string(),
        "memcp".to_string(),
    )
    .await
    .expect("cmd_ingest_from_reader stdin");
    assert_eq!(value["summary"]["stored"], 2, "body: {value}");
    assert_eq!(value["results"].as_array().unwrap().len(), 2);
}

/// INGEST-06 / D-20: CLI `memcp ingest --message '{...}'` one-shot.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_cli_ingest_message_flag(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();

    // Wrap a single-message JSON object using the same helper `cmd_ingest` uses
    // for the --message flag path. A single JSON object is NOT a valid JSONL-
    // auto-detected batch by itself (well, it is — JSONL with one object); but
    // the --message flag always goes via a dedicated single-object parse.
    let jsonl = "{\"role\":\"user\",\"content\":\"hello-from-flag\"}\n";
    let value = memcp::cli::cmd_ingest_from_reader(
        &store,
        &config,
        std::io::Cursor::new(jsonl),
        "cli-test".to_string(),
        "cli-flag-sess".to_string(),
        "memcp".to_string(),
    )
    .await
    .expect("cmd_ingest_from_reader single-message");
    assert_eq!(value["summary"]["stored"], 1, "body: {value}");
    assert_eq!(value["results"].as_array().unwrap().len(), 1);
    assert_eq!(value["results"][0]["status"], "stored");
}

// ---------------------------------------------------------------------------
// D-02 boot-safety gate (flipped in Plan 24.5-02)
// ---------------------------------------------------------------------------

/// D-02: Daemon boot fails when non-loopback bind and no ingest key configured.
#[test]
fn test_boot_fails_non_loopback_no_key() {
    use memcp::transport::boot_safety::check_ingest_auth_safety;

    assert!(check_ingest_auth_safety("127.0.0.1:8080", None).is_ok());
    assert!(check_ingest_auth_safety("::1", None).is_ok());
    assert!(check_ingest_auth_safety("localhost", None).is_ok());

    let err0 = check_ingest_auth_safety("0.0.0.0:8080", None);
    assert!(err0.is_err(), "0.0.0.0 with no key must refuse boot");
    let msg_txt = err0.unwrap_err();
    assert!(
        msg_txt.contains("MEMCP_INGEST__API_KEY"),
        "error message must name the env var, got: {msg_txt}"
    );

    assert!(check_ingest_auth_safety("0.0.0.0:8080", Some("k")).is_ok());
    assert!(check_ingest_auth_safety("192.168.1.5", None).is_err());
}

// ---------------------------------------------------------------------------
// Migration 027 — reply_to_id column
// ---------------------------------------------------------------------------

/// Migration 027: `reply_to_id` column exists, is nullable TEXT, and the partial index exists.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_reply_to_id_migration(pool: sqlx::PgPool) {
    use sqlx::Row;

    let row = sqlx::query(
        "SELECT data_type, is_nullable FROM information_schema.columns \
         WHERE table_name = 'memories' AND column_name = 'reply_to_id'",
    )
    .fetch_one(&pool)
    .await
    .expect("information_schema should list reply_to_id column after migration 027");

    let data_type: String = row.try_get("data_type").unwrap();
    let is_nullable: String = row.try_get("is_nullable").unwrap();
    assert_eq!(data_type, "text", "reply_to_id should be TEXT");
    assert_eq!(is_nullable, "YES", "reply_to_id should be nullable");

    let idx_row = sqlx::query(
        "SELECT indexname FROM pg_indexes \
         WHERE tablename = 'memories' AND indexname = 'idx_memories_reply_to_id'",
    )
    .fetch_optional(&pool)
    .await
    .expect("pg_indexes query should succeed");

    assert!(
        idx_row.is_some(),
        "idx_memories_reply_to_id partial index should exist after migration 027"
    );
}
