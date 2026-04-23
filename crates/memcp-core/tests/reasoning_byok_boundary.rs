//! Phase 25 Plan 08: BYOK transport middleware security tests.
//!
//! D-08 hardening (Critical Failure Mode #5) + Reviews HIGH #2 (Ollama no-auth).
//! In-process `Router::oneshot` harness — no TCP bind, no background task.
//!
//! Coverage:
//!   1. test_pro_tier_strips_caller_api_key_header     — T-25-08-01
//!   2. test_byok_tier_requires_headers                — D-08 non-ollama BYOK
//!   3. test_unknown_provider_rejected                 — allowlist 400
//!   4. test_pro_with_server_key_absent_returns_503    — Pro no-env for non-ollama
//!   5. test_no_reasoning_header_passes_through        — non-reasoning routes untouched
//!   6. test_byok_extracts_caller_key                  — BYOK happy path
//!   7. test_byok_ollama_no_api_key_required           — HIGH #2 BYOK ollama
//!   8. test_pro_ollama_no_env_key_succeeds            — HIGH #2 Pro ollama

use std::collections::HashMap;

use axum::{
    body::Body,
    response::IntoResponse,
    routing::get,
    Extension, Json, Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::util::ServiceExt;

use memcp::intelligence::reasoning::ProviderCredentials;
use memcp::transport::api::reasoning::{require_reasoning_creds, ReasoningMwState};
use memcp::transport::health::{ReasoningCreds, ReasoningTenancy};

/// Handler that echoes whatever `ProviderCredentials` the middleware inserted
/// into request extensions. Returns `{"no_extension": true}` when no creds
/// were attached (non-reasoning request that fell through pass-through path).
async fn echo_creds(ext: Option<Extension<ProviderCredentials>>) -> impl IntoResponse {
    match ext {
        Some(Extension(creds)) => Json(json!({
            "api_key": creds.api_key,
            "base_url": creds.base_url,
        })),
        None => Json(json!({"api_key": null, "base_url": null, "no_extension": true})),
    }
}

/// Non-reasoning route: verifies pass-through on missing x-reasoning-provider.
async fn nothing() -> impl IntoResponse {
    Json(json!({"msg": "no reasoning ctx"}))
}

fn app(tenancy: ReasoningTenancy, env_keys: HashMap<String, String>) -> Router {
    let state = ReasoningMwState {
        tenancy,
        creds: ReasoningCreds { env_keys },
    };
    Router::new()
        .route("/echo", get(echo_creds))
        .route("/none", get(nothing))
        .layer(axum::middleware::from_fn_with_state(
            state,
            require_reasoning_creds,
        ))
}

async fn get_body(router: Router, req: axum::http::Request<Body>) -> (u16, Value) {
    let resp = router.oneshot(req).await.expect("router oneshot");
    let status = resp.status().as_u16();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("body collect")
        .to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

fn req_with(headers: &[(&str, &str)], path: &str) -> axum::http::Request<Body> {
    let mut builder = axum::http::Request::builder().uri(path);
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    builder.body(Body::empty()).expect("valid request")
}

// ---------------------------------------------------------------------------
// 1. D-08: Pro tenancy strips caller-supplied x-reasoning-api-key.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_pro_tier_strips_caller_api_key_header() {
    let mut env = HashMap::new();
    env.insert("kimi".into(), "SERVER_ENV_KEY".into());
    let router = app(ReasoningTenancy::Pro, env);

    let r = req_with(
        &[
            ("x-reasoning-provider", "kimi"),
            ("x-reasoning-api-key", "ROGUE_KEY"),
        ],
        "/echo",
    );
    let (status, body) = get_body(router, r).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["api_key"].as_str(),
        Some("SERVER_ENV_KEY"),
        "Pro tier must use server env key, NOT caller-supplied ROGUE_KEY (D-08 hardening)"
    );
}

// ---------------------------------------------------------------------------
// 2. BYOK non-ollama missing x-reasoning-api-key -> 401.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_byok_tier_requires_headers() {
    let router = app(ReasoningTenancy::Byok, HashMap::new());
    let r = req_with(&[("x-reasoning-provider", "openai")], "/echo");
    let (status, _body) = get_body(router, r).await;
    assert_eq!(
        status, 401,
        "BYOK missing x-reasoning-api-key (non-ollama) must 401"
    );
}

// ---------------------------------------------------------------------------
// 3. Unknown provider name -> 400 Bad Request.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_unknown_provider_rejected() {
    let router = app(ReasoningTenancy::Pro, HashMap::new());
    let r = req_with(
        &[
            ("x-reasoning-provider", "anthropic"),
            ("x-reasoning-api-key", "x"),
        ],
        "/echo",
    );
    let (status, body) = get_body(router, r).await;
    assert_eq!(status, 400);
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("unknown reasoning provider"));
}

// ---------------------------------------------------------------------------
// 4. Pro + non-ollama + no server env key -> 503 Service Unavailable.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_pro_with_server_key_absent_returns_503() {
    // We simulate Pro tenancy explicitly even though env_keys is empty —
    // this test exercises the middleware branch, not the tenancy() derivation
    // (which would return Byok for an empty env map).
    let router = app(ReasoningTenancy::Pro, HashMap::new());
    let r = req_with(&[("x-reasoning-provider", "kimi")], "/echo");
    let (status, _body) = get_body(router, r).await;
    assert_eq!(
        status, 503,
        "Pro with no server env key for non-ollama provider must 503"
    );
}

// ---------------------------------------------------------------------------
// 5. No x-reasoning-provider header -> middleware passes through untouched.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_no_reasoning_header_passes_through() {
    let router = app(ReasoningTenancy::Pro, HashMap::new());
    let r = req_with(&[], "/none");
    let (status, body) = get_body(router, r).await;
    assert_eq!(status, 200);
    assert_eq!(body["msg"].as_str(), Some("no reasoning ctx"));
}

// ---------------------------------------------------------------------------
// 6. BYOK + non-ollama + valid caller key -> creds.api_key == header value.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_byok_extracts_caller_key() {
    let router = app(ReasoningTenancy::Byok, HashMap::new());
    let r = req_with(
        &[
            ("x-reasoning-provider", "openai"),
            ("x-reasoning-api-key", "sk-caller-123"),
        ],
        "/echo",
    );
    let (status, body) = get_body(router, r).await;
    assert_eq!(status, 200);
    assert_eq!(body["api_key"].as_str(), Some("sk-caller-123"));
}

// ---------------------------------------------------------------------------
// 7. HIGH #2: BYOK + ollama without x-reasoning-api-key -> 200, api_key=None.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_byok_ollama_no_api_key_required() {
    let router = app(ReasoningTenancy::Byok, HashMap::new());
    let r = req_with(&[("x-reasoning-provider", "ollama")], "/echo");
    let (status, body) = get_body(router, r).await;
    assert_eq!(
        status, 200,
        "BYOK + ollama without api-key must succeed (ollama is no-auth)"
    );
    assert!(
        body["api_key"].is_null(),
        "ollama credentials must carry api_key=None, got: {body}"
    );
}

// ---------------------------------------------------------------------------
// 8. HIGH #2: Pro tenancy (flipped to Pro by some other provider's env key)
//    + ollama request with no MEMCP_REASONING__OLLAMA_API_KEY -> 200,
//    api_key=None. Asserts T-25-08-07 regression guard.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_pro_ollama_no_env_key_succeeds() {
    let mut env = HashMap::new();
    env.insert("kimi".into(), "SERVER_KIMI_KEY".into()); // flips tenancy() to Pro
    // Intentionally NO "ollama" entry.
    let router = app(ReasoningTenancy::Pro, env);

    let r = req_with(&[("x-reasoning-provider", "ollama")], "/echo");
    let (status, body) = get_body(router, r).await;
    assert_eq!(
        status, 200,
        "Pro + ollama with no env key must succeed (no-auth)"
    );
    assert!(
        body["api_key"].is_null(),
        "ollama credentials in Pro tenancy without env key must carry api_key=None, got: {body}"
    );
}
