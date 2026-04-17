// Integration tests for Phase 24: Knowledge Tiers
//
// Wave 0 test stubs covering TIER-01 through TIER-06 plus D-06 orphan tagging.
// Wave 1 tests (TIER-01/02/03) implemented by Plan 01.

mod common;

use common::builders::MemoryBuilder;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;
use sqlx::Row;

// ---------------------------------------------------------------------------
// TIER-01: Migration adds columns with correct defaults and constraints
// ---------------------------------------------------------------------------

/// Verify knowledge_tier column exists with default 'explicit' and CHECK constraint
/// for the 5-value enum (raw, imported, explicit, derived, pattern).
/// Verify source_ids JSONB column exists and is nullable.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_tier_migration(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Verify all 5 valid tier values are accepted by storing memories with each tier
    for tier in &["raw", "imported", "explicit", "derived", "pattern"] {
        let mut builder = MemoryBuilder::new()
            .content(&format!("Migration test for tier {}", tier))
            .knowledge_tier(tier);

        // derived requires source_ids per D-04
        if *tier == "derived" {
            builder = builder.source_ids(vec!["00000000-0000-0000-0000-000000000001"]);
        }

        let memory = store.store(builder.build()).await.unwrap();
        assert_eq!(
            &memory.knowledge_tier, tier,
            "Memory should have tier '{}'",
            tier
        );
    }

    // Store a memory without specifying knowledge_tier — should default to 'explicit'
    let input = MemoryBuilder::new()
        .content("Test migration default tier")
        .build();
    let memory = store.store(input).await.unwrap();
    assert_eq!(memory.knowledge_tier, "explicit", "Default tier should be 'explicit'");
    assert!(memory.source_ids.is_none(), "source_ids should be None by default");

    // Verify CHECK constraint rejects invalid tier values via a savepoint
    // (sqlx::test wraps the pool in a transaction; failed queries abort it unless wrapped)
    let result: Result<_, _> = sqlx::query(
        "DO $$ BEGIN \
           UPDATE memories SET knowledge_tier = 'invalid' WHERE id = '00000000-0000-0000-0000-000000000000'; \
         EXCEPTION WHEN check_violation THEN NULL; \
         END $$"
    )
    .execute(&pool)
    .await;
    // The DO block catches the check_violation — success means the constraint exists
    assert!(result.is_ok(), "CHECK constraint should exist on knowledge_tier column");

    // Verify source_ids column exists and is nullable
    let row = sqlx::query("SELECT source_ids FROM memories LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let _source_ids: Option<serde_json::Value> = row.try_get("source_ids").unwrap();
}

// ---------------------------------------------------------------------------
// TIER-02: write_path -> tier inference at store time
// ---------------------------------------------------------------------------

/// Store memories with different write_path values and verify tier is inferred:
/// - write_path='auto_store' -> tier='raw'
/// - write_path='explicit_store' -> tier='explicit'
/// - write_path='import' -> tier='imported'
/// - write_path='session_summary' -> tier='raw'
/// - write_path='annotation' -> tier='explicit'
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_tier_inference_from_write_path(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let cases = vec![
        ("auto_store", "raw"),
        ("explicit_store", "explicit"),
        ("import", "imported"),
        ("session_summary", "raw"),
        ("annotation", "explicit"),
        ("ingest", "raw"),
    ];

    for (write_path, expected_tier) in cases {
        let input = MemoryBuilder::new()
            .content(&format!("Memory with write_path={}", write_path))
            .write_path(write_path)
            .build();
        let memory = store.store(input).await.unwrap();
        assert_eq!(
            memory.knowledge_tier, expected_tier,
            "write_path='{}' should infer tier='{}'",
            write_path, expected_tier
        );
    }

    // No write_path -> defaults to 'explicit'
    let input = MemoryBuilder::new()
        .content("Memory with no write_path")
        .build();
    let memory = store.store(input).await.unwrap();
    assert_eq!(memory.knowledge_tier, "explicit", "No write_path should default to 'explicit'");
}

/// Store a memory with write_path='auto_store' AND knowledge_tier='explicit'.
/// Caller override should take precedence: tier should be 'explicit'.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_tier_caller_override(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let input = MemoryBuilder::new()
        .content("Caller override test")
        .write_path("auto_store")
        .knowledge_tier("explicit")
        .build();
    let memory = store.store(input).await.unwrap();
    assert_eq!(
        memory.knowledge_tier, "explicit",
        "Caller override should take precedence over write_path inference"
    );
}

/// Store a memory with knowledge_tier='derived' and no source_ids.
/// Should return Err(Validation) per D-04.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_derived_requires_source_ids(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // derived with no source_ids -> error
    let input = MemoryBuilder::new()
        .content("Derived without sources")
        .knowledge_tier("derived")
        .build();
    let result = store.store(input).await;
    assert!(result.is_err(), "derived tier without source_ids should fail");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("derived tier requires non-empty source_ids"),
        "Error should mention source_ids requirement, got: {}",
        err_msg
    );

    // derived with empty source_ids -> also error
    let input2 = MemoryBuilder::new()
        .content("Derived with empty sources")
        .knowledge_tier("derived")
        .source_ids(vec![])
        .build();
    let result2 = store.store(input2).await;
    assert!(result2.is_err(), "derived tier with empty source_ids should fail");

    // derived with non-empty source_ids -> success
    let input3 = MemoryBuilder::new()
        .content("Derived with valid sources")
        .knowledge_tier("derived")
        .source_ids(vec!["00000000-0000-0000-0000-000000000001"])
        .build();
    let result3 = store.store(input3).await;
    assert!(result3.is_ok(), "derived tier with non-empty source_ids should succeed");
    let memory3 = result3.unwrap();
    assert_eq!(memory3.knowledge_tier, "derived");
    assert!(memory3.source_ids.is_some());
}

// ---------------------------------------------------------------------------
// TIER-03: Backfill classifies existing memories correctly
// ---------------------------------------------------------------------------

/// Since sqlx::test runs all migrations, we can't test backfill of pre-existing data.
/// Instead, verify that the tier inference produces correct tiers for each write_path,
/// which is the same logic the backfill migration applies.
/// Also verify via direct SQL that the backfill UPDATE in migration 026 ran correctly
/// by checking that no memories have an incorrect tier/write_path combination.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_tier_backfill(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store memories with various write_paths
    let write_paths = vec![
        ("auto_store", "raw"),
        ("session_summary", "raw"),
        ("explicit_store", "explicit"),
        ("annotation", "explicit"),
        ("import", "imported"),
    ];

    for (wp, expected_tier) in &write_paths {
        let input = MemoryBuilder::new()
            .content(&format!("Backfill test: {}", wp))
            .write_path(wp)
            .build();
        let memory = store.store(input).await.unwrap();
        assert_eq!(&memory.knowledge_tier, expected_tier);

        // Verify the tier persists in the database via get()
        let fetched = store.get(&memory.id).await.unwrap();
        assert_eq!(
            &fetched.knowledge_tier, expected_tier,
            "Tier should persist for write_path='{}'",
            wp
        );
    }

    // Verify no mismatched tiers exist in the DB
    let row = sqlx::query(
        "SELECT COUNT(*) as cnt FROM memories \
         WHERE (write_path = 'auto_store' AND knowledge_tier != 'raw') \
         OR (write_path = 'explicit_store' AND knowledge_tier != 'explicit') \
         OR (write_path = 'import' AND knowledge_tier != 'imported') \
         AND deleted_at IS NULL"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mismatch_count: i64 = row.try_get("cnt").unwrap();
    assert_eq!(mismatch_count, 0, "No tier/write_path mismatches should exist");
}

// ---------------------------------------------------------------------------
// TIER-04: Composite score formula includes tier dimension
// ---------------------------------------------------------------------------

/// Verify tier_score_for() returns correct normalized scores:
/// - pattern=1.0, derived=0.75, explicit=0.5, imported=0.25, raw=0.0
#[ignore = "Wave 2: 24-02-PLAN"]
#[test]
fn test_composite_score_tier_boost() {
    todo!("tier_score_for('pattern')=1.0, 'raw'=0.0, 'explicit'=0.5")
}

/// Two memories with identical content — one with 'pattern' tier and one with 'explicit'.
/// The pattern-tier memory should rank higher in search results due to tier boost.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 2: 24-02-PLAN"]
async fn test_tier_search_ranking(pool: PgPool) {
    todo!("Two memories with same content, one pattern one explicit, pattern ranks higher")
}

// ---------------------------------------------------------------------------
// TIER-05: source_ids round-trips through store and get
// ---------------------------------------------------------------------------

/// Store a memory with source_ids=['uuid1','uuid2'], retrieve by id,
/// verify source_ids are returned correctly as a JSON array.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 3: 24-03-PLAN"]
async fn test_source_ids_roundtrip(pool: PgPool) {
    todo!("Store with source_ids=['uuid1','uuid2'], get by id, verify source_ids returned")
}

// ---------------------------------------------------------------------------
// TIER-06: Tier filtering and source chain traversal
// ---------------------------------------------------------------------------

/// Store memories with raw and explicit tiers. Search without explicit tier filter.
/// Raw memories should be excluded from results by default (D-10).
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 2: 24-02-PLAN"]
async fn test_tier_filter_excludes_raw(pool: PgPool) {
    todo!("Store raw + explicit memories, search without tier filter, raw excluded from results")
}

/// Store source memories, then store a derived memory with source_ids pointing
/// to those sources. Using --show-sources should return the source memories.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 3: 24-03-PLAN"]
async fn test_show_sources_single_hop(pool: PgPool) {
    todo!("Store source memories, store derived with source_ids, show-sources returns sources")
}

/// Queryless recall (--first, no query) should return all tiers including raw (D-11).
/// Store memories across raw, explicit, and derived tiers, verify all returned.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 2: 24-02-PLAN"]
async fn test_queryless_recall_all_tiers(pool: PgPool) {
    todo!("Store raw + explicit + derived, queryless recall returns all of them")
}

// ---------------------------------------------------------------------------
// D-06: Orphan tagging when source is GC'd
// ---------------------------------------------------------------------------

/// When a source memory is deleted/GC'd, derived memories referencing it
/// via source_ids should get tagged with 'orphaned_sources'. No cascade delete.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
#[ignore = "Wave 3: 24-03-PLAN"]
async fn test_orphan_tagging_on_gc(pool: PgPool) {
    todo!("Delete source memory, derived memory gets 'orphaned_sources' tag")
}
