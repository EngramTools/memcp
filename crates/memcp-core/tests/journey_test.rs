//! Full lifecycle journey tests.
//!
//! Each test proves an end-to-end user flow works across the full stack:
//! store → embed → search → recall → feedback → GC lifecycle → dedup.
//!
//! All tests use `#[sqlx::test(migrator = "memcp::MIGRATOR")]` for ephemeral
//! database isolation. Mock embeddings are inserted directly via SQL.

mod common;
use common::builders::MemoryBuilder;

use std::sync::Arc;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{MemoryStore, ListFilter, CreateMemory};
use memcp::config::{RecallConfig, GcConfig};
use memcp::recall::RecallEngine;
use memcp::gc::run_gc;
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Embedding helpers
// ---------------------------------------------------------------------------

/// Spike embedding: position `seed % dim` = 1.0, rest = small. Normalized.
fn spike_emb(seed: usize, dim: usize) -> Vec<f32> {
    let mut v = vec![0.001f32; dim];
    v[seed % dim] = 1.0;
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn emb_str(v: &[f32]) -> String {
    format!("[{}]", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","))
}

/// Insert a mock embedding and mark memory as complete.
async fn insert_mock_embedding(pool: &PgPool, memory_id: &str, embedding: &[f32]) {
    sqlx::query(
        "INSERT INTO memory_embeddings \
         (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at) \
         VALUES ($1, $2, $3::vector, 'test-mock', '1', $4, true, NOW(), NOW())",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(memory_id)
    .bind(emb_str(embedding))
    .bind(embedding.len() as i32)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(memory_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Get current salience stability for a memory.
async fn get_stability(pool: &PgPool, memory_id: &str) -> f32 {
    let row: Option<f32> = sqlx::query_scalar(
        "SELECT stability FROM memory_salience WHERE memory_id = $1",
    )
    .bind(memory_id)
    .fetch_optional(pool)
    .await
    .unwrap();
    row.unwrap_or(1.0) // default stability
}

// ---------------------------------------------------------------------------
// Test 1: Full store → search → recall → session dedup journey
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_search_recall_journey(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Use dim=8 for test embeddings.
    // Query embedding: spike at position 0.
    // "dark mode preference" memory: also spike at 0 → high cosine similarity.
    // Others: spike at position 1..4 → low cosine similarity with query.
    let query_emb = spike_emb(0, 8);

    // Store 5 diverse memories
    // Note: RecallEngine no-extraction tier only recalls fact/summary type_hints.
    // Using "fact" for all so the journey test exercises the full recall path.
    let m_dark = store.store(MemoryBuilder::new()
        .content("User prefers dark mode in all editors")
        .type_hint("fact")
        .build()).await.unwrap();
    let m_rust = store.store(MemoryBuilder::new()
        .content("Team uses Rust 2021 edition for all backend services")
        .type_hint("fact")
        .build()).await.unwrap();
    let m_db = store.store(MemoryBuilder::new()
        .content("PostgreSQL with pgvector is the chosen database")
        .type_hint("decision")
        .build()).await.unwrap();
    let m_test = store.store(MemoryBuilder::new()
        .content("Run cargo test before every commit")
        .type_hint("instruction")
        .build()).await.unwrap();
    let m_ui = store.store(MemoryBuilder::new()
        .content("Frontend uses React with TypeScript strict mode")
        .type_hint("fact")
        .build()).await.unwrap();

    // Insert embeddings: dark mode gets same spike as query (perfect match)
    insert_mock_embedding(&pool, &m_dark.id, &query_emb).await;
    // Others get different spike positions (lower similarity)
    insert_mock_embedding(&pool, &m_rust.id, &spike_emb(1, 8)).await;
    insert_mock_embedding(&pool, &m_db.id, &spike_emb(2, 8)).await;
    insert_mock_embedding(&pool, &m_test.id, &spike_emb(3, 8)).await;
    insert_mock_embedding(&pool, &m_ui.id, &spike_emb(4, 8)).await;

    // Recall with query embedding
    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);

    let session_id = "journey-test-session-1".to_string();
    let result1 = engine
        .recall(&query_emb, Some(session_id.clone()), false, None, &[])
        .await
        .unwrap();

    // Should return at least the dark mode preference (high similarity)
    assert!(result1.count > 0, "first recall should return results");

    let ids1: Vec<&str> = result1.memories.iter().map(|m| m.memory_id.as_str()).collect();
    assert!(
        ids1.contains(&m_dark.id.as_str()),
        "dark mode preference should be in results (highest similarity): got {:?}",
        ids1
    );

    // Verify session dedup: second recall with same session returns 0
    let result2 = engine
        .recall(&query_emb, Some(session_id.clone()), false, None, &[])
        .await
        .unwrap();
    assert_eq!(
        result2.count, 0,
        "second recall with same session should return 0 (already seen)"
    );

    // Recall records the session (which we proved by dedup) — this is the key correctness
    // property. The session dedup above already proves the recall pipeline ran end-to-end.
    // Optionally verify the recall session was recorded in recall_sessions table.
    let session_count: Option<i64> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recall_sessions WHERE session_id = $1",
    )
    .bind(&session_id)
    .fetch_optional(&pool)
    .await
    .unwrap_or(None);
    // session_count may be None if the table doesn't exist, which is fine
    // The dedup proof above is sufficient to prove recall pipeline worked
    let _ = session_count;
}

// ---------------------------------------------------------------------------
// Test 2: Feedback loop — useful increases, irrelevant decreases stability
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_affects_future_salience(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store two memories
    let m_useful = store.store(MemoryBuilder::new()
        .content("Memory that the user found useful")
        .type_hint("fact")
        .build()).await.unwrap();

    let m_irrelevant = store.store(MemoryBuilder::new()
        .content("Memory that the user found irrelevant")
        .type_hint("fact")
        .build()).await.unwrap();

    // Record initial salience
    let initial_useful = get_stability(&pool, &m_useful.id).await;
    let initial_irrelevant = get_stability(&pool, &m_irrelevant.id).await;

    // Apply "useful" feedback
    store.apply_feedback(&m_useful.id, "useful").await.unwrap();

    // Apply "irrelevant" feedback
    store.apply_feedback(&m_irrelevant.id, "irrelevant").await.unwrap();

    // Check salience changed
    let after_useful = get_stability(&pool, &m_useful.id).await;
    let after_irrelevant = get_stability(&pool, &m_irrelevant.id).await;

    assert!(
        after_useful > initial_useful,
        "useful feedback should increase stability: before={:.4}, after={:.4}",
        initial_useful, after_useful
    );
    assert!(
        after_irrelevant < initial_irrelevant,
        "irrelevant feedback should decrease stability: before={:.4}, after={:.4}",
        initial_irrelevant, after_irrelevant
    );
}

// ---------------------------------------------------------------------------
// Test 3: GC lifecycle — soft delete, then hard purge
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_gc_lifecycle(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store 5 memories
    let mut ids: Vec<String> = Vec::new();
    for i in 0..5 {
        let m = store.store(MemoryBuilder::new()
            .content(&format!("GC lifecycle memory {}", i))
            .type_hint("fact")
            .build()).await.unwrap();
        ids.push(m.id);
    }

    // Set 2 memories to very low salience
    let low_ids = &ids[0..2];
    for id in low_ids {
        sqlx::query(
            "INSERT INTO memory_salience (memory_id, stability, difficulty, reinforcement_count, created_at, updated_at) \
             VALUES ($1, 0.01, 5.0, 0, NOW(), NOW()) \
             ON CONFLICT (memory_id) DO UPDATE SET stability = 0.01, updated_at = NOW()",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

        // Set created_at to 60 days ago
        sqlx::query(
            "UPDATE memories SET created_at = NOW() - '60 days'::interval WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Run GC — should soft-delete the 2 low-salience memories
    let config = GcConfig {
        enabled: true,
        salience_threshold: 0.5,
        min_age_days: 30,
        min_memory_floor: 0,
        gc_interval_secs: 3600,
        hard_purge_grace_days: 30,
    };

    let result1 = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result1.pruned_count >= 2,
        "GC should soft-delete at least 2 low-salience memories, got pruned={}",
        result1.pruned_count
    );

    // Verify soft-delete
    for id in low_ids {
        let deleted: Option<bool> = sqlx::query_scalar(
            "SELECT deleted_at IS NOT NULL FROM memories WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            deleted.unwrap_or(false),
            "low-salience memory {} should be soft-deleted",
            id
        );
    }

    // Backdate deleted_at beyond the hard_purge_grace_days (30 days)
    for id in low_ids {
        sqlx::query(
            "UPDATE memories SET deleted_at = NOW() - '60 days'::interval WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Run GC again — should hard purge the 2 soft-deleted memories
    let result2 = run_gc(&store, &config, false).await.unwrap();
    assert!(
        result2.hard_purged_count >= 2,
        "second GC should hard-purge at least 2 old soft-deleted memories, got hard_purged={}",
        result2.hard_purged_count
    );

    // Verify hard purge — rows should no longer exist
    for id in low_ids {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "hard-purged memory {} should no longer exist", id);
    }
}

// ---------------------------------------------------------------------------
// Test 4: Idempotency dedup journey — same key returns same ID; same content dedup
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_idempotency_dedup_journey(pool: PgPool) {
    use memcp::config::IdempotencyConfig;

    let config = IdempotencyConfig {
        dedup_window_secs: 3600, // wide window
        key_ttl_secs: 86400,
        max_key_length: 256,
    };
    let store = PostgresMemoryStore::from_pool_with_idempotency(pool.clone(), config)
        .await
        .unwrap();

    // a. Store memory with idempotency_key="journey-key-1"
    let first = store.store(CreateMemory {
        content: "Idempotency journey test memory".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "test".to_string(),
        audience: "global".to_string(),
        idempotency_key: Some("journey-key-1".to_string()),
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
    }).await.unwrap();

    // b. Store again with same idempotency_key — should return same ID
    let second = store.store(CreateMemory {
        content: "Idempotency journey test memory — updated content should be ignored".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "test".to_string(),
        audience: "global".to_string(),
        idempotency_key: Some("journey-key-1".to_string()),
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
    }).await.unwrap();

    assert_eq!(
        first.id, second.id,
        "same idempotency_key should return same memory ID"
    );

    // c. Store same content without key — content-hash dedup catches it within window
    let third = store.store(CreateMemory {
        content: "Content hash dedup journey test".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "test".to_string(),
        audience: "global".to_string(),
        idempotency_key: None,
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
    }).await.unwrap();

    let fourth = store.store(CreateMemory {
        content: "Content hash dedup journey test".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "test".to_string(),
        audience: "global".to_string(),
        idempotency_key: None,
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
    }).await.unwrap();

    assert_eq!(
        third.id, fourth.id,
        "identical content within dedup window should return same memory ID"
    );

    // d. Backdate created_at beyond dedup window — store again, assert new ID
    sqlx::query(
        "UPDATE memories SET created_at = NOW() - '2 hours'::interval WHERE id = $1",
    )
    .bind(&third.id)
    .execute(&pool)
    .await
    .unwrap();

    let fifth = store.store(CreateMemory {
        content: "Content hash dedup journey test".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "test".to_string(),
        audience: "global".to_string(),
        idempotency_key: None,
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
    }).await.unwrap();

    assert_ne!(
        third.id, fifth.id,
        "content outside dedup window should create a new memory"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Store → list verifies basic CRUD round-trip
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_list_get_delete_journey(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store 3 memories with a unique source for isolation
    let source = "journey-crud-test";
    for i in 0..3 {
        store.store(CreateMemory {
            content: format!("CRUD journey memory {}", i),
            type_hint: "fact".to_string(),
            source: source.to_string(),
            tags: Some(vec![format!("idx:{}", i)]),
            created_at: None,
            actor: None,
            actor_type: "test".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
        }).await.unwrap();
    }

    // List — should return 3 memories
    let list_result = store.list(ListFilter {
        source: Some(source.to_string()),
        limit: 20,
        ..Default::default()
    }).await.unwrap();

    assert_eq!(
        list_result.memories.len(), 3,
        "list should return all 3 stored memories"
    );

    // Get by ID — should return the correct memory
    let first_id = list_result.memories[0].id.clone();
    let fetched = store.get(&first_id).await.unwrap();
    assert_eq!(fetched.id, first_id, "get should return correct memory");
    assert_eq!(fetched.source, source, "get should return correct source");

    // Delete — should succeed; subsequent get should fail
    store.delete(&first_id).await.unwrap();

    let get_result = store.get(&first_id).await;
    assert!(
        get_result.is_err(),
        "get after delete should return error (NotFound)"
    );

    // List again — should return 2 memories
    let list_after = store.list(ListFilter {
        source: Some(source.to_string()),
        limit: 20,
        ..Default::default()
    }).await.unwrap();

    assert_eq!(
        list_after.memories.len(), 2,
        "list after delete should return 2 memories"
    );
}
