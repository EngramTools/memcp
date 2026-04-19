//! POST /v1/ingest handler + shared ingest pipeline entry (Phase 24.5).
//!
//! See `.planning/phases/24.5-universal-ingestion-api/24.5-03-PLAN.md`.
//! The handler is added in Task 2; this module currently exposes the
//! `make_idempotency_key` helper used by both the handler and callers that
//! need a stable client-side hash.

use sha2::{Digest, Sha256};

/// D-13: Deterministic SHA-256 idempotency key over (source, session_id, timestamp, role, content).
///
/// Fields are length-prefixed (LE u32) before hashing so that `(source="ab", session="c")`
/// and `(source="a", session="bc")` cannot collide via boundary ambiguity (RESEARCH Topic 2).
/// Stable across daemon restarts and across Rust compiler versions.
pub fn make_idempotency_key(
    source: &str,
    session_id: &str,
    timestamp: chrono::DateTime<chrono::Utc>,
    role: &str,
    content: &str,
) -> String {
    let mut hasher = Sha256::new();
    for field in &[source, session_id, role, content] {
        hasher.update((field.len() as u32).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    let ts = timestamp.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    hasher.update((ts.len() as u32).to_le_bytes());
    hasher.update(ts.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}
