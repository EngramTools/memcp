//! Unit stubs for `make_idempotency_key` (Phase 24.5 / D-13).
//!
//! These tests pin the three non-negotiable properties of the idempotency hash:
//!   1. Determinism — same inputs produce the same key on every call.
//!   2. Sensitivity — changing any single input field changes the key.
//!   3. No naive-concatenation collisions — length-prefixing each field prevents
//!      `(source="ab", session="c")` from colliding with `(source="a", session="bc")`.
//!
//! All stubs are `#[ignore]`d until Plan 24.5-03 lands `make_idempotency_key`.
//! RESEARCH Topic 2 specifies the length-prefixing requirement.

// TODO(24.5-03): uncomment once make_idempotency_key is introduced.
// use memcp::transport::api::ingest::make_idempotency_key;

/// D-13: Two calls with identical inputs must return the same string.
#[ignore = "24.5-03 impl pending"]
#[test]
fn test_idempotency_key_stable() {
    unimplemented!("24.5-03");
}

/// D-13: Calls with one field changed (e.g. different role) must return different strings.
#[ignore = "24.5-03 impl pending"]
#[test]
fn test_idempotency_key_differs() {
    unimplemented!("24.5-03");
}

/// D-13 / RESEARCH Topic 2: Length-prefixing prevents naive-concat collisions.
///
/// Canonical collision case (naive `source + session + ...`):
///   - (source="ab", session="c", ...) → "abc..."
///   - (source="a",  session="bc", ...) → "abc..."
/// Both tuples must hash to DIFFERENT keys under the length-prefixed scheme.
#[ignore = "24.5-03 impl pending"]
#[test]
fn test_idempotency_key_length_prefix_collision() {
    unimplemented!("24.5-03");
}
