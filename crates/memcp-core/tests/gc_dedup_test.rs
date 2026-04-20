//! GC + dedup integration tests.
//!
//! Tests salience-based pruning, TTL expiry, min_memory_floor protection, dry-run,
//! hard purge, and content-hash deduplication. All use `#[sqlx::test]` for
//! ephemeral databases.

mod common;
use common::builders::MemoryBuilder;

use memcp::config::{GcConfig, IdempotencyConfig};
use memcp::gc::run_gc;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;

/// Set low stability for a memory (override whatever was set by store()).
async fn set_stability(pool: &PgPool, memory_id: &str, stability: f32) {
    sqlx::query(
        "INSERT INTO memory_salience (memory_id, stability, difficulty, reinforcement_count, created_at, updated_at)
         VALUES ($1, $2, 5.0, 0, NOW(), NOW())
         ON CONFLICT (memory_id) DO UPDATE SET stability = EXCLUDED.stability, updated_at = NOW()",
    )
    .bind(memory_id)
    .bind(stability)
    .execute(pool)
    .await
    .unwrap();
}

/// Set a memory's created_at to a past timestamp for min_age_days testing.
async fn set_created_at_days_ago(pool: &PgPool, memory_id: &str, days_ago: i64) {
    sqlx::query("UPDATE memories SET created_at = NOW() - ($1 || ' days')::interval WHERE id = $2")
        .bind(days_ago.to_string())
        .bind(memory_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Returns true if the memory has been soft-deleted (deleted_at IS NOT NULL).
async fn is_soft_deleted(pool: &PgPool, memory_id: &str) -> bool {
    let row: Option<bool> =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM memories WHERE id = $1")
            .bind(memory_id)
            .fetch_optional(pool)
            .await
            .unwrap();
    row.unwrap_or(false)
}

/// Returns true if the memory row no longer exists (hard purged).
async fn is_hard_purged(pool: &PgPool, memory_id: &str) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE id = $1")
        .bind(memory_id)
        .fetch_one(pool)
        .await
        .unwrap();
    count == 0
}

// ---------------------------------------------------------------------------
// Test 1: GC prunes low-salience memories
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_prunes_low_salience(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store 5 memories, make 2 have low stability AND be old enough
    let mut ids = Vec::new();
    for i in 0..5 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("GC prune test memory {}", i))
                    .build(),
            )
            .await
            .unwrap();
        ids.push(m.id);
    }

    // Make all memories 2 days old (past min_age_days=1)
    for id in &ids {
        set_created_at_days_ago(&pool, id, 2).await;
    }

    // Set 2 memories to very low stability
    set_stability(&pool, &ids[0], 0.05).await;
    set_stability(&pool, &ids[1], 0.05).await;
    // Others keep default stability (1.0)

    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.5, // threshold: stability < 0.5 = candidate
        min_age_days: 1,         // 1 day old minimum
        min_memory_floor: 0,     // no floor (0 so GC runs)
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    let result = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result.pruned_count >= 2,
        "should prune at least 2 low-salience memories"
    );

    // Verify soft-delete
    assert!(
        is_soft_deleted(&pool, &ids[0]).await,
        "low-salience memory 0 should be soft-deleted"
    );
    assert!(
        is_soft_deleted(&pool, &ids[1]).await,
        "low-salience memory 1 should be soft-deleted"
    );

    // High-salience memories should NOT be deleted
    assert!(
        !is_soft_deleted(&pool, &ids[2]).await,
        "high-salience memory 2 should NOT be deleted"
    );
    assert!(
        !is_soft_deleted(&pool, &ids[3]).await,
        "high-salience memory 3 should NOT be deleted"
    );
    assert!(
        !is_soft_deleted(&pool, &ids[4]).await,
        "high-salience memory 4 should NOT be deleted"
    );
}

// ---------------------------------------------------------------------------
// Test 2: GC respects min_age_days — recent memories are not pruned
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_respects_min_age(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store memories with recent created_at (default is NOW())
    let mut ids = Vec::new();
    for i in 0..3 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Recent memory {}", i))
                    .build(),
            )
            .await
            .unwrap();
        ids.push(m.id);
    }

    // Set all to very low stability
    for id in &ids {
        set_stability(&pool, id, 0.05).await;
    }

    // Memories are only ~seconds old; min_age_days=30 should protect them
    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.5,
        min_age_days: 30,
        min_memory_floor: 0,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    let result = run_gc(&store, &config, false).await.unwrap();
    assert_eq!(
        result.pruned_count, 0,
        "GC should not prune recent memories even if low-salience"
    );

    for id in &ids {
        assert!(
            !is_soft_deleted(&pool, id).await,
            "recent memory should NOT be soft-deleted"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: min_memory_floor — GC skips when count is at or below floor
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_min_memory_floor(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store 3 memories (below any reasonable floor)
    let mut ids = Vec::new();
    for i in 0..3 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Floor test memory {}", i))
                    .build(),
            )
            .await
            .unwrap();
        ids.push(m.id);
        set_stability(&pool, &ids[i], 0.01).await;
        set_created_at_days_ago(&pool, &ids[i], 60).await;
    }

    // Set floor=100 — we only have 3 memories, so GC must skip
    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.5,
        min_age_days: 0,
        min_memory_floor: 100,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    let result = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result.skipped_reason.is_some(),
        "GC should return a skip reason when below floor"
    );
    let reason = result.skipped_reason.unwrap();
    assert!(
        reason.contains("floor") || reason.contains("below"),
        "skip reason should mention floor: {}",
        reason
    );
}

// ---------------------------------------------------------------------------
// Test 4: TTL expiry — memories past expires_at are soft-deleted
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_ttl_expiry(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store a memory and set expires_at to yesterday
    let m = store
        .store(
            MemoryBuilder::new()
                .content("TTL expiry test memory")
                .build(),
        )
        .await
        .unwrap();

    sqlx::query("UPDATE memories SET expires_at = NOW() - '1 day'::interval WHERE id = $1")
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();

    // Store another memory with no expiry (should not be touched)
    let m2 = store
        .store(MemoryBuilder::new().content("No TTL memory").build())
        .await
        .unwrap();

    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.1, // Very low threshold — only expired should be pruned
        min_age_days: 0,
        min_memory_floor: 0,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    let result = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result.expired_count >= 1,
        "should expire at least 1 TTL-expired memory"
    );
    assert!(
        is_soft_deleted(&pool, &m.id).await,
        "TTL-expired memory should be soft-deleted"
    );
    assert!(
        !is_soft_deleted(&pool, &m2.id).await,
        "non-expired memory should NOT be soft-deleted"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Hard purge — old soft-deleted memories are hard-purged
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_hard_purge(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store 2 memories: one will be soft-deleted (to purge), one live (to avoid floor skip)
    let m_live = store
        .store(
            MemoryBuilder::new()
                .content("Live memory for floor")
                .build(),
        )
        .await
        .unwrap();
    let m_delete = store
        .store(
            MemoryBuilder::new()
                .content("Hard purge test memory")
                .build(),
        )
        .await
        .unwrap();

    // Soft-delete m_delete with deleted_at 60 days ago (past grace period of 30 days)
    sqlx::query("UPDATE memories SET deleted_at = NOW() - '60 days'::interval WHERE id = $1")
        .bind(&m_delete.id)
        .execute(&pool)
        .await
        .unwrap();

    // Live count is 1 (m_live), floor=0 so GC won't skip
    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.1,
        min_age_days: 0,
        min_memory_floor: 0,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30, // 30-day grace period
    };

    let result = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result.hard_purged_count >= 1,
        "should hard-purge at least 1 old soft-deleted memory, got hard_purged_count={}",
        result.hard_purged_count
    );
    assert!(
        is_hard_purged(&pool, &m_delete.id).await,
        "hard-purged memory should no longer exist in DB"
    );
    assert!(
        !is_hard_purged(&pool, &m_live.id).await,
        "live memory should still exist"
    );
    let _ = m_live; // suppress unused warning
}

// ---------------------------------------------------------------------------
// Test 6: Dry-run — candidates returned but nothing deleted
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_dry_run(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store memories with low salience and old age
    let mut ids = Vec::new();
    for i in 0..3 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Dry run test memory {}", i))
                    .build(),
            )
            .await
            .unwrap();
        ids.push(m.id.clone());
        set_stability(&pool, &m.id, 0.05).await;
        set_created_at_days_ago(&pool, &m.id, 10).await;
    }

    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.5,
        min_age_days: 1,
        min_memory_floor: 0,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    // dry_run=true: should return candidates but NOT delete
    let result = run_gc(&store, &config, true).await.unwrap();
    assert!(
        result.pruned_count > 0 || result.candidates.len() > 0,
        "dry-run should report candidates"
    );

    // Verify nothing was actually deleted
    for id in &ids {
        assert!(
            !is_soft_deleted(&pool, id).await,
            "dry-run should NOT soft-delete memories"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 7: Content-hash dedup within window — identical content returns same ID
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_content_hash_dedup(pool: PgPool) {
    let idempotency_config = IdempotencyConfig {
        dedup_window_secs: 3600, // Wide window — both stores happen within it
        key_ttl_secs: 86400,
        max_key_length: 256,
    };
    let store = PostgresMemoryStore::from_pool_with_idempotency(pool, idempotency_config)
        .await
        .unwrap();

    let content = "Content hash dedup test — identical content";

    let first = store
        .store(MemoryBuilder::new().content(content).build())
        .await
        .unwrap();

    // Store same content within the dedup window — should return existing memory
    let second = store
        .store(MemoryBuilder::new().content(content).build())
        .await
        .unwrap();

    assert_eq!(
        first.id, second.id,
        "identical content within dedup window should return same memory ID"
    );
}

// ---------------------------------------------------------------------------
// Phase 24.75-03: dedup with no chunk-sibling logic. Two semantically similar
// memories (no parent relationship because chunks are gone) must merge directly.
// This locks in the Plan 02 simplification: dedup.rs::process_job has exactly
// one candidate-selection path (candidates[0]) — no find_non_sibling branch.
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_dedup_no_sibling_logic(pool: PgPool) {
    use memcp::config::DedupConfig;
    use memcp::gc::dedup::{DedupJob, DedupWorker};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Two semantically similar memories with DIFFERENT content strings (so
    // content-hash dedup in store() doesn't short-circuit). No parent/sibling
    // relationship exists — chunk_removal landed in Plan 02.
    let older = store
        .store(
            MemoryBuilder::new()
                .content("The quick brown fox jumps over the lazy dog in the morning sun (version A)")
                .build(),
        )
        .await
        .unwrap();
    let newer = store
        .store(
            MemoryBuilder::new()
                .content("The quick brown fox jumps over the lazy dog in the morning sun (version B)")
                .build(),
        )
        .await
        .unwrap();

    // Embeddings won't be populated by `store()` — write identical vectors
    // directly so the similarity search fires. `find_similar_memories` requires
    // `embedding_status = 'complete'` so we flip both memories.
    let vector = pgvector::Vector::from(vec![0.1f32; 384]);
    for id in [&older.id, &newer.id] {
        sqlx::query(
            "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, tier, created_at, updated_at)
             VALUES ($1, $2, $3::vector, 'test-model', 'v1', 384, true, 'fast', NOW(), NOW())
             ON CONFLICT DO NOTHING",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(id)
        .bind(&vector)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Run the dedup worker with a job for the newer memory.
    let (tx, rx) = mpsc::channel(1);
    let cfg = DedupConfig {
        enabled: true,
        similarity_threshold: 0.9,
    };
    tx.send(DedupJob {
        memory_id: newer.id.clone(),
        embedding: vector,
    })
    .await
    .unwrap();
    drop(tx); // close channel so worker exits after processing

    let worker = DedupWorker::new(store.clone(), cfg, rx);
    worker.run().await;

    // Post-merge: newer is soft-deleted, older is untouched. dedup.rs picks
    // candidates[0] directly — no sibling-skip branch.
    let newer_deleted: Option<bool> =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM memories WHERE id = $1")
            .bind(&newer.id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    let older_deleted: Option<bool> =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM memories WHERE id = $1")
            .bind(&older.id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        newer_deleted,
        Some(true),
        "newer memory should be soft-deleted by dedup merge"
    );
    assert_eq!(
        older_deleted,
        Some(false),
        "older memory stays live as the merge target"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Content-hash dedup outside window — identical content creates new memory
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_content_hash_dedup_outside_window(pool: PgPool) {
    let idempotency_config = IdempotencyConfig {
        dedup_window_secs: 1, // 1-second window
        key_ttl_secs: 86400,
        max_key_length: 256,
    };
    let store = PostgresMemoryStore::from_pool_with_idempotency(pool.clone(), idempotency_config)
        .await
        .unwrap();

    let content = "Content hash dedup outside window test";

    let first = store
        .store(MemoryBuilder::new().content(content).build())
        .await
        .unwrap();

    // Manually backdate created_at so it's outside the 1-second window.
    // The dedup query checks `created_at > NOW() - window`, so backdating
    // created_at on the memories table is sufficient.
    sqlx::query("UPDATE memories SET created_at = NOW() - '1 hour'::interval WHERE id = $1")
        .bind(&first.id)
        .execute(&pool)
        .await
        .unwrap();

    // Store same content again — should create new memory (outside window)
    let second = store
        .store(MemoryBuilder::new().content(content).build())
        .await
        .unwrap();

    assert_ne!(
        first.id, second.id,
        "identical content outside dedup window should create a new memory"
    );
}
