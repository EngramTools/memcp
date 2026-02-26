//! Embedding pipeline lifecycle tests.
//!
//! Tests status transitions (pending → complete), search exclusion of pending
//! embeddings, and idempotency key dedup. Uses `#[sqlx::test]` for isolated
//! ephemeral databases.

mod common;
use common::builders::MemoryBuilder;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{MemoryStore, SearchFilter};
use memcp::config::IdempotencyConfig;
use sqlx::PgPool;
use pgvector::Vector;

/// Format embedding as postgres vector literal
fn emb_str(emb: &[f32]) -> String {
    format!("[{}]", emb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","))
}

/// Insert a mock current embedding for a memory.
async fn insert_mock_embedding(pool: &PgPool, memory_id: &str, emb: &[f32]) {
    use uuid::Uuid;
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at)
         VALUES ($1, $2, $3::vector, 'test-mock', '1', $4, true, $5, $5)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(memory_id)
    .bind(emb_str(emb))
    .bind(emb.len() as i32)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(memory_id)
        .execute(pool)
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: store() creates memory with embedding_status='pending'
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_creates_pending_embedding(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let m = store
        .store(MemoryBuilder::new().content("Pending embedding test").build())
        .await
        .unwrap();

    assert_eq!(
        m.embedding_status, "pending",
        "newly stored memory should have embedding_status='pending'"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Embedding status transitions: pending → processing → complete
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_embedding_status_transitions(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let m = store
        .store(MemoryBuilder::new().content("Status transition test").build())
        .await
        .unwrap();
    assert_eq!(m.embedding_status, "pending");

    // Transition to 'processing'
    store.update_embedding_status(&m.id, "processing").await.unwrap();
    let updated = store.get(&m.id).await.unwrap();
    assert_eq!(updated.embedding_status, "processing");

    // Transition to 'complete'
    store.update_embedding_status(&m.id, "complete").await.unwrap();
    let done = store.get(&m.id).await.unwrap();
    assert_eq!(done.embedding_status, "complete");
}

// ---------------------------------------------------------------------------
// Test 3: Search excludes pending embeddings
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_search_excludes_pending_embeddings(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let query_emb: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0];

    // Store memory with done embedding — should appear in search
    let done_mem = store
        .store(
            MemoryBuilder::new()
                .content("Memory with complete embedding")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &done_mem.id, &query_emb).await;

    // Store memory without embedding (pending) — should NOT appear in search
    let pending_mem = store
        .store(
            MemoryBuilder::new()
                .content("Memory with pending embedding")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    // Do NOT insert embedding — stays pending

    let filter = SearchFilter {
        query_embedding: Vector::from(query_emb.clone()),
        limit: 10,
        ..Default::default()
    };
    let result = store.search_similar(&filter).await.unwrap();

    let result_ids: Vec<&str> = result.hits.iter().map(|h| h.memory.id.as_str()).collect();
    assert!(
        result_ids.contains(&done_mem.id.as_str()),
        "complete-embedding memory should appear in search results"
    );
    assert!(
        !result_ids.contains(&pending_mem.id.as_str()),
        "pending-embedding memory should NOT appear in search results"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Idempotency key dedup — same key returns same memory ID
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_idempotency_key_dedup(pool: PgPool) {
    // Use a very wide dedup window for the test
    let idempotency_config = IdempotencyConfig {
        dedup_window_secs: 3600,
        key_ttl_secs: 86400,
        max_key_length: 256,
    };
    let store = PostgresMemoryStore::from_pool_with_idempotency(pool, idempotency_config)
        .await
        .unwrap();

    let first = store
        .store(
            MemoryBuilder::new()
                .content("Idempotency test memory")
                .idempotency_key("test-key-1")
                .build(),
        )
        .await
        .unwrap();

    // Store same key again — should return the same memory
    let second = store
        .store(
            MemoryBuilder::new()
                .content("Different content, same key")
                .idempotency_key("test-key-1")
                .build(),
        )
        .await
        .unwrap();

    assert_eq!(
        first.id, second.id,
        "second store with same idempotency_key should return same memory ID"
    );
}
