//! Unit tests for `make_idempotency_key` (Phase 24.5 / D-13).
//!
//! These tests pin the three non-negotiable properties of the idempotency hash:
//!   1. Determinism — same inputs produce the same key on every call.
//!   2. Sensitivity — changing any single input field changes the key.
//!   3. No naive-concatenation collisions — length-prefixing each field prevents
//!      `(source="ab", session="c")` from colliding with `(source="a", session="bc")`.
//!
//! RESEARCH Topic 2 specifies the length-prefixing requirement.

use chrono::{TimeZone, Utc};
use memcp::transport::api::ingest::make_idempotency_key;

/// D-13: Two calls with identical inputs must return the same string.
#[test]
fn test_idempotency_key_stable() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
    let a = make_idempotency_key("telegram-bot", "sess-1", ts, "user", "hello");
    let b = make_idempotency_key("telegram-bot", "sess-1", ts, "user", "hello");
    assert_eq!(a, b, "identical inputs must produce identical keys");
    // Sanity: output is 64-char lowercase hex (SHA-256 digest).
    assert_eq!(a.len(), 64, "expected 64-char hex digest, got {}", a.len());
    assert!(
        a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "expected lowercase hex, got {a}"
    );
}

/// D-13: Calls with one field changed (e.g. different role) must return different strings.
#[test]
fn test_idempotency_key_differs() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
    let user_key = make_idempotency_key("telegram-bot", "sess-1", ts, "user", "hello");
    let assistant_key = make_idempotency_key("telegram-bot", "sess-1", ts, "assistant", "hello");
    assert_ne!(
        user_key, assistant_key,
        "changing role must change the key"
    );
}

/// D-13 / RESEARCH Topic 2: Length-prefixing prevents naive-concat collisions.
///
/// Canonical collision case (naive `source + session + ...`):
///   - (source="ab", session="c",  ...) naive -> "abc..."
///   - (source="a",  session="bc", ...) naive -> "abc..."
/// Both tuples must hash to DIFFERENT keys under the length-prefixed scheme.
#[test]
fn test_idempotency_key_length_prefix_collision() {
    let ts = Utc.with_ymd_and_hms(2026, 4, 19, 12, 0, 0).unwrap();
    let k1 = make_idempotency_key("ab", "c", ts, "r", "x");
    let k2 = make_idempotency_key("a", "bc", ts, "r", "x");
    assert_ne!(
        k1, k2,
        "length-prefixed hash must distinguish boundary-ambiguous inputs"
    );
}
