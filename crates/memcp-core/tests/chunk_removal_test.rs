//! Phase 24.75 chunk removal — migration 028 integration tests.
//!
//! RED scaffolds pre-registered by Plan 24.75-00. Each test is registered as
//! ignored with an annotation pointing at the downstream plan that un-ignores
//! it:
//!
//!   - test_migration_028_collapse            → 24.75-01 (orchestrator)
//!   - test_migration_028_refuses_unreassembled → 24.75-03 (DDL guardrail)
//!   - test_columns_dropped                   → 24.75-03 (column drop)
//!
//! Targets CHUNK-02 (migration end-to-end) + CHUNK-03 (column drop) from
//! 24.75-RESEARCH.md's Validation Architecture. Mirrors Phase 24.5 Plan 00's
//! pattern: tests compile (so `cargo test --no-run` is green) and pass
//! vacuously until the owning plan lands.

#![allow(clippy::panic)]

use sqlx::PgPool;

// ---------------------------------------------------------------------------
// CHUNK-02 — migration 028 end-to-end (orchestrator + re-embed)
// ---------------------------------------------------------------------------

/// Seeds a parent memory with 3 chunk rows, invokes the migration-028
/// orchestrator helper, and asserts:
///   1. `memories WHERE parent_id IS NOT NULL` returns 0 rows after.
///   2. The parent's embedding row has `is_current = true` after re-embed.
///
/// The orchestrator helper is expected to land in Plan 24.75-01 Task 2 —
/// the test stays ignored with that downstream-plan marker until then.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "pending 24.75-01"]
async fn test_migration_028_collapse(_pool: PgPool) {
    unimplemented!("orchestrator lands in 24.75-01");
}

/// After migration 028 DDL, attempting to run the orchestrator with chunk rows
/// still present (manually re-inserted, simulating a partial rollback) MUST
/// fail with a clean, actionable error — not silently drop data.
///
/// This guardrail flips ON in Plan 24.75-03 Task 3 once the DDL step owns
/// the "no chunks present" precondition.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "pending 24.75-03"]
async fn test_migration_028_refuses_unreassembled(_pool: PgPool) {
    unimplemented!("DDL guardrail lands in 24.75-03");
}

// ---------------------------------------------------------------------------
// CHUNK-03 — column drop (parent_id, chunk_index, total_chunks)
// ---------------------------------------------------------------------------

/// After migration 028 DDL applies, the chunk columns (parent_id, chunk_index,
/// total_chunks) no longer exist on `memories`. A `SELECT parent_id FROM
/// memories LIMIT 0` must surface a column-not-found error from Postgres.
///
/// Flips ON in Plan 24.75-03 Task 3 with the ALTER TABLE DROP COLUMN DDL.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "pending 24.75-03"]
async fn test_columns_dropped(_pool: PgPool) {
    unimplemented!("column drop lands in 24.75-03");
}
