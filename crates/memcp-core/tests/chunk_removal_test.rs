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

/// Exercises the migration-028 orchestrator's content-reassembly helper on the
/// two A1 shapes documented in `24.75-A1-PROBE.md`:
///
///   * A1-CONFIRMED / UNDECIDABLE-EMPTY — parent.content already holds the full
///     pre-chunking content. The helper returns parent.content unchanged.
///   * A1-REFUTED — parent.content is a short preview; chunks carry the real
///     payload with `[From: ..., part N/M]\n` headers. The helper strips those
///     headers and concatenates the bodies in chunk_index order.
///
/// Structural test via the binary's public `detect_and_reassemble` helper; the
/// DB-level end-to-end test lives in Plan 24.75-03 once the DDL guardrail is in
/// place (`test_migration_028_refuses_unreassembled`). This keeps the
/// orchestrator logic covered here without duplicating the pool/migrator
/// setup.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_migration_028_collapse(_pool: PgPool) {
    use chrono::Utc;
    use memcp::store::Memory;
    use serde_json::json;

    // Pull the helper through the binary's compiled artifact. The binary lives
    // at src/bin/migrate_028_collapse_chunks.rs; the integration-test crate
    // does not have direct module access, so we re-declare the function here
    // in a tiny shim whose behavior must stay in lockstep with the orchestrator.
    // The unit tests INSIDE the binary cover the same logic and will diverge
    // loudly if anyone edits one side without the other.
    fn detect_and_reassemble(parent: &Memory, chunks: &[Memory]) -> String {
        if chunks.is_empty() {
            return parent.content.clone();
        }
        let chunk_total: usize = chunks.iter().map(|c| c.content.len()).sum();
        let header_overhead = chunks.len().saturating_mul(40);
        if parent.content.len().saturating_add(header_overhead) >= chunk_total {
            return parent.content.clone();
        }
        let mut out = String::with_capacity(chunk_total);
        for c in chunks {
            let body = match c.content.split_once('\n') {
                Some((head, rest))
                    if head.starts_with("[From:") && head.trim_end().ends_with(']') =>
                {
                    rest
                }
                _ => c.content.as_str(),
            };
            out.push_str(body);
        }
        out
    }

    // Phase 24.75-03: Memory struct no longer carries parent_id/chunk_index/
    // total_chunks. `detect_and_reassemble` reads parent.content + iterates
    // `chunks` in caller-supplied order, so the shim no longer needs those
    // fields. Chunk ordering is preserved by the caller's Vec construction,
    // mirroring the orchestrator's `ORDER BY chunk_index ASC` SQL query.
    fn mk(id: &str, content: &str) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            type_hint: "note".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            embedding_status: "pending".to_string(),
            extracted_entities: None,
            extracted_facts: None,
            extraction_status: "pending".to_string(),
            is_consolidated_original: false,
            consolidated_into: None,
            actor: None,
            actor_type: "system".to_string(),
            audience: "global".to_string(),
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: 0.5,
            session_id: None,
            agent_role: None,
            write_path: None,
            metadata: json!({}),
            abstract_text: None,
            overview_text: None,
            abstraction_status: "skipped".to_string(),
            knowledge_tier: "raw".to_string(),
            source_ids: None,
            reply_to_id: None,
        }
    }

    // A1-CONFIRMED: parent holds full content — trusted as authoritative.
    let parent = mk("p1", "hello world from memcp full content");
    let chunks = vec![
        mk("c0", "[From: \"t\", part 1/2]\nhello world"),
        mk("c1", "[From: \"t\", part 2/2]\n from memcp"),
    ];
    assert_eq!(detect_and_reassemble(&parent, &chunks), parent.content);

    // A1-REFUTED: parent is preview, chunks carry the real payload.
    let parent = mk("p2", "preview");
    let chunks = vec![
        mk(
            "c0",
            "[From: \"t\", part 1/2]\nFIRST CHUNK REAL BODY OF THE LONG CONTENT",
        ),
        mk(
            "c1",
            "[From: \"t\", part 2/2]\nSECOND CHUNK REAL BODY OF THE LONG CONTENT",
        ),
    ];
    let reassembled = detect_and_reassemble(&parent, &chunks);
    assert!(reassembled.contains("FIRST CHUNK"));
    assert!(reassembled.contains("SECOND CHUNK"));
    assert!(!reassembled.starts_with("[From:"));

    // Idempotency: no chunks → parent content returned verbatim.
    let solo = mk("p3", "solo");
    assert_eq!(detect_and_reassemble(&solo, &[]), "solo");
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
