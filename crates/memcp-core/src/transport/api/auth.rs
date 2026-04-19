//! API key auth middleware — checks X-Memcp-Key or Authorization: Bearer against a configured key.
//!
//! Designed as a reusable `from_fn_with_state` layer. First auth middleware in memcp (Phase 24.5).
//! Phase 12 (multi-tenant auth) will extend or replace the extraction logic but keep the
//! same application point.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

/// Shared auth state — the configured API key (if any).
///
/// `None` => middleware is a passthrough (D-02 loopback no-key mode; boot-safety gate in
/// `transport/daemon.rs` guarantees this is only reachable on a loopback bind).
#[derive(Clone, Default)]
pub struct AuthState {
    pub api_key: Option<Arc<String>>,
}

impl AuthState {
    /// Build an `AuthState` from an optional string key.
    pub fn from_optional(key: Option<String>) -> Self {
        Self {
            api_key: key.map(Arc::new),
        }
    }
}

/// Axum middleware that rejects requests lacking a matching API key.
///
/// Accepts either `X-Memcp-Key: <key>` or `Authorization: Bearer <key>` (D-01).
/// Uses constant-time byte-wise compare to avoid timing oracles (T-24.5-02).
pub async fn require_api_key(
    State(auth): State<AuthState>,
    req: Request,
    next: Next,
) -> Response {
    // D-02: when no key is configured, the middleware is a passthrough.
    let Some(expected) = auth.api_key.as_deref() else {
        return next.run(req).await;
    };

    let headers = req.headers();

    // Priority 1: X-Memcp-Key
    let provided = headers
        .get("x-memcp-key")
        .and_then(|v| v.to_str().ok())
        // Priority 2: Authorization: Bearer <key>
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(|s| s.trim())
        });

    let valid = provided
        .map(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
        .unwrap_or(false);

    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    next.run(req).await
}

/// Constant-time byte-wise equality (T-24.5-02). Never short-circuits on first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_equal() {
        assert!(constant_time_eq(b"secret", b"secret"));
    }

    #[test]
    fn constant_time_eq_different() {
        assert!(!constant_time_eq(b"secret", b"SECRET"));
        assert!(!constant_time_eq(b"secret", b"secre"));
        assert!(!constant_time_eq(b"secret", b"secrett"));
    }

    #[test]
    fn constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
