//! Integration tests for `consolidation::similarity::find_similar_memories()`.
//!
//! Uses `#[sqlx::test]` for ephemeral database isolation. Each test manually inserts
//! embeddings into `memory_embeddings` to avoid requiring the fastembed runtime.

mod common;
use common::builders::MemoryBuilder;

use chrono;
use memcp::consolidation::similarity::find_similar_memories;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;
use uuid;

/// Create a 384-dimensional base vector with a recognizable pattern.
fn make_base_vector() -> Vec<f32> {
    let mut v = vec![0.0f32; 384];
    for i in 0..10 {
        v[i] = 1.0 / (i as f32 + 1.0);
    }
    v
}

/// Create a vector very similar to the base (tiny perturbation on dim 0).
fn make_similar_vector() -> Vec<f32> {
    let mut v = make_base_vector();
    v[0] += 0.01;
    v
}

/// Create a vector orthogonal to the base (non-overlapping dimensions).
fn make_orthogonal_vector() -> Vec<f32> {
    let mut v = vec![0.0f32; 384];
    for i in 10..20 {
        v[i] = 1.0 / (i as f32 + 1.0);
    }
    v
}

/// Helper: insert an embedding row and mark the memory as embedding_status='complete'.
async fn insert_embedding(pool: &PgPool, memory_id: &str, embedding: Vec<f32>) {
    let dim = embedding.len() as i32;
    let vec = pgvector::Vector::from(embedding);
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();

    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at)
         VALUES ($1, $2, $3, 'test', '1.0', $4, TRUE, $5, $6)",
    )
    .bind(&id)
    .bind(memory_id)
    .bind(&vec)
    .bind(dim)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert embedding");

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(memory_id)
        .execute(pool)
        .await
        .expect("update embedding_status");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_find_similar_above_threshold(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem_a = store
        .store(MemoryBuilder::new().content("Memory A about Rust").build())
        .await
        .unwrap();
    let mem_b = store
        .store(MemoryBuilder::new().content("Memory B about Rust").build())
        .await
        .unwrap();

    let base = make_base_vector();
    let similar = make_similar_vector();

    insert_embedding(&pool, &mem_a.id, base.clone()).await;
    insert_embedding(&pool, &mem_b.id, similar).await;

    let base_vec = pgvector::Vector::from(base);
    let results = find_similar_memories(&pool, &mem_a.id, &base_vec, 0.8, 10)
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "should find 1 similar memory");
    assert_eq!(results[0].memory_id, mem_b.id);
    assert!(
        results[0].similarity > 0.8,
        "similarity {} should be > 0.8",
        results[0].similarity
    );
    assert_eq!(results[0].content, "Memory B about Rust");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_find_similar_excludes_self(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem = store
        .store(MemoryBuilder::new().content("Only memory").build())
        .await
        .unwrap();

    let base = make_base_vector();
    insert_embedding(&pool, &mem.id, base.clone()).await;

    let base_vec = pgvector::Vector::from(base);
    let results = find_similar_memories(&pool, &mem.id, &base_vec, 0.0, 10)
        .await
        .unwrap();

    assert!(
        results.is_empty(),
        "self should be excluded, got {} results",
        results.len()
    );
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_find_similar_empty_below_threshold(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem_a = store
        .store(MemoryBuilder::new().content("Memory A").build())
        .await
        .unwrap();
    let mem_b = store
        .store(MemoryBuilder::new().content("Memory B").build())
        .await
        .unwrap();

    let base = make_base_vector();
    let ortho = make_orthogonal_vector();

    insert_embedding(&pool, &mem_a.id, base.clone()).await;
    insert_embedding(&pool, &mem_b.id, ortho).await;

    let base_vec = pgvector::Vector::from(base);
    let results = find_similar_memories(&pool, &mem_a.id, &base_vec, 0.99, 10)
        .await
        .unwrap();

    assert!(
        results.is_empty(),
        "orthogonal vectors should not meet 0.99 threshold, got {} results",
        results.len()
    );
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_find_similar_excludes_consolidated_originals(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem_a = store
        .store(MemoryBuilder::new().content("Memory A").build())
        .await
        .unwrap();
    let mem_b = store
        .store(
            MemoryBuilder::new()
                .content("Memory B consolidated")
                .build(),
        )
        .await
        .unwrap();

    let base = make_base_vector();
    let similar = make_similar_vector();

    insert_embedding(&pool, &mem_a.id, base.clone()).await;
    insert_embedding(&pool, &mem_b.id, similar).await;

    // Mark mem_b as a consolidated original
    sqlx::query("UPDATE memories SET is_consolidated_original = TRUE WHERE id = $1")
        .bind(&mem_b.id)
        .execute(&pool)
        .await
        .expect("mark consolidated original");

    let base_vec = pgvector::Vector::from(base);
    let results = find_similar_memories(&pool, &mem_a.id, &base_vec, 0.0, 10)
        .await
        .unwrap();

    assert!(
        results.is_empty(),
        "consolidated originals should be excluded, got {} results",
        results.len()
    );
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_find_similar_respects_limit(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let source = store
        .store(MemoryBuilder::new().content("Source memory").build())
        .await
        .unwrap();
    let _m1 = store
        .store(MemoryBuilder::new().content("Similar 1").build())
        .await
        .unwrap();
    let _m2 = store
        .store(MemoryBuilder::new().content("Similar 2").build())
        .await
        .unwrap();

    let base = make_base_vector();
    insert_embedding(&pool, &source.id, base.clone()).await;

    // Use slight perturbations for each similar memory
    let mut sim1 = make_base_vector();
    sim1[0] += 0.01;
    insert_embedding(&pool, &_m1.id, sim1).await;

    let mut sim2 = make_base_vector();
    sim2[1] += 0.01;
    insert_embedding(&pool, &_m2.id, sim2).await;

    let base_vec = pgvector::Vector::from(base);
    let results = find_similar_memories(&pool, &source.id, &base_vec, 0.5, 1)
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "limit=1 should return exactly 1 result");
}
