//! BYOK + Pro-tier credential middleware for reasoning endpoints (Phase 25 Plan 08).
//!
//! D-08 hardening (Critical Failure Mode #5): Pro-tier requests MUST ignore any
//! caller-supplied `x-reasoning-api-key` header — the middleware strips it
//! BEFORE `next.run()` and tracing-warns the event (never the key value).
//!
//! Reviews HIGH #2: Ollama is no-auth. A `provider=ollama` request is
//! short-circuited before tenancy branching — no API key required in either
//! tenancy, and `ProviderCredentials::api_key` is `None` when the server has no
//! env key configured (the Ollama adapter tolerates `None` since the upstream
//! is keyless).
//!
//! Reviews MEDIUM #8: wiring sites are enumerated in `25-08-PLAN.md`. This
//! module exposes `require_reasoning_creds` + `ReasoningMwState`; the router
//! layers it in `transport/api/mod.rs::router` AFTER the Phase 24.5
//! `require_api_key` auth layer (auth stays outermost per 24.5-03 decision).

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::intelligence::reasoning::ProviderCredentials;
use crate::transport::health::{ReasoningCreds, ReasoningTenancy};

/// Closed allowlist of reasoning providers for Phase 25. Any other value →
/// 400 Bad Request. Comparison is performed on the lowercased trimmed header.
const ALLOWED_PROVIDERS: &[&str] = &["kimi", "openai", "ollama"];

/// State passed to `require_reasoning_creds` via `from_fn_with_state`. Clone-cheap:
/// `ReasoningTenancy` is `Copy` and `ReasoningCreds` wraps a small `HashMap`.
#[derive(Clone)]
pub struct ReasoningMwState {
    pub tenancy: ReasoningTenancy,
    pub creds: ReasoningCreds,
}

/// Axum middleware enforcing reasoning credential policy.
///
/// Flow:
/// 1. No `x-reasoning-provider` header → pass through (not a reasoning request).
/// 2. Provider not in allowlist → 400.
/// 3. Provider == "ollama" → no-auth short-circuit. Strip any caller key on Pro
///    tenancy (defense-in-depth + warn log), then insert `ProviderCredentials`
///    with optional env key and continue.
/// 4. Pro + non-ollama → strip caller `x-reasoning-api-key` (always, warn if
///    present), look up server env key; missing → 503.
/// 5. BYOK + non-ollama → require `x-reasoning-api-key` header; missing → 401.
///
/// Credentials are inserted into request extensions as `ProviderCredentials`;
/// downstream handlers extract via `axum::Extension<ProviderCredentials>`.
/// T-25-08-02: NEVER logs the `api_key` value — only event names + provider.
pub async fn require_reasoning_creds(
    State(state): State<ReasoningMwState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let headers = req.headers().clone();
    let Some(provider_hdr) = headers
        .get("x-reasoning-provider")
        .and_then(|v| v.to_str().ok())
    else {
        // No reasoning header — pass through so non-reasoning routes are unaffected.
        return next.run(req).await;
    };

    // T-25-08-04: case-fold + trim so "Kimi" and " kimi " both hit the allowlist.
    let provider = provider_hdr.trim().to_ascii_lowercase();
    if !ALLOWED_PROVIDERS.contains(&provider.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "unknown reasoning provider",
                "provided": provider,
            })),
        )
            .into_response();
    }

    // Reviews HIGH #2: Ollama short-circuit — no API key required in either tenancy.
    if provider == "ollama" {
        let api_key = state.creds.env_keys.get("ollama").cloned();
        if state.tenancy == ReasoningTenancy::Pro
            && headers.contains_key("x-reasoning-api-key")
        {
            tracing::warn!(
                provider = %provider,
                event = "pro_tier_stripped_byok_headers_ollama",
                "Pro-tier ollama request supplied x-reasoning-api-key; stripped (ollama is no-auth)"
            );
            req.headers_mut().remove("x-reasoning-api-key");
        }
        let creds = ProviderCredentials {
            api_key,
            base_url: None,
        };
        req.extensions_mut().insert(creds);
        return next.run(req).await;
    }

    // Non-ollama path: enforce tenancy policy.
    let creds = match state.tenancy {
        ReasoningTenancy::Pro => {
            // T-25-08-01: strip caller key unconditionally on Pro tenancy. Warn
            // only when one was present so clean Pro requests stay quiet.
            let had_key = headers.contains_key("x-reasoning-api-key");
            if had_key {
                tracing::warn!(
                    provider = %provider,
                    event = "pro_tier_stripped_byok_headers",
                    "Pro-tier request supplied x-reasoning-api-key; stripped before downstream dispatch"
                );
            }
            req.headers_mut().remove("x-reasoning-api-key");

            let Some(env_key) = state.creds.env_keys.get(&provider).cloned() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": "Pro-tier: server has no credentials for provider",
                        "provider": provider,
                    })),
                )
                    .into_response();
            };
            ProviderCredentials {
                api_key: Some(env_key),
                base_url: None,
            }
        }
        ReasoningTenancy::Byok => {
            let Some(key) = headers
                .get("x-reasoning-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
            else {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": "BYOK-tier: x-reasoning-api-key header required",
                        "provider": provider,
                    })),
                )
                    .into_response();
            };
            // T-25-01-01 SSRF mitigation: base_url is always None on BYOK;
            // never accept a caller-supplied base URL.
            ProviderCredentials {
                api_key: Some(key),
                base_url: None,
            }
        }
    };

    req.extensions_mut().insert(creds);
    next.run(req).await
}
