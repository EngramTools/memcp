//! Phase 24.75 chunk removal — get_memory_span ranking + offset tests.
//!
//! RED scaffolds pre-registered by Plan 24.75-00. Both tests stay ignored
//! against the 24.75-04 marker — flipped on once the `get_memory_span` tool
//! ships in Plan 24.75-04.
//!
//! Targets CHUNK-04 (topic-ranked span retrieval + valid byte offsets) from
//! 24.75-RESEARCH.md's Validation Architecture.

#![allow(clippy::panic)]

use sqlx::PgPool;

/// Create a ~5kB memory whose content covers three topics (authentication,
/// billing, shipping), call `get_memory_span(id, "authentication")`, and
/// assert the returned span:
///   1. `content` substring contains the auth paragraph and NOT the billing
///      or shipping paragraphs.
///   2. `span.start` and `span.end` point into the auth paragraph's byte
///      range within the parent memory's content.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "pending 24.75-04"]
async fn test_topic_ranked_span(_pool: PgPool) {
    unimplemented!("get_memory_span lands in 24.75-04");
}

/// Structural invariants on any span returned by `get_memory_span`:
///   - `0 <= span.start < span.end <= memory.content.len()`
///   - `memory.content[span.start..span.end] == returned.content`
///
/// These hold regardless of topic ranking, so they stay green even if the
/// ranker's exact output shifts. Flips ON alongside test_topic_ranked_span
/// in Plan 24.75-04.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "pending 24.75-04"]
async fn test_memory_span_offsets_valid(_pool: PgPool) {
    unimplemented!("get_memory_span lands in 24.75-04");
}
