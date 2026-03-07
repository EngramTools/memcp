//! Integration tests for discover_associations() — cosine sweet-spot discovery.
//!
//! Each test uses `#[sqlx::test(migrator = "memcp::MIGRATOR")]` for a fresh,
//! isolated ephemeral database. Mock embeddings are inserted directly via SQL —
//! no fastembed dependency required.
//!
//! The key insight: cosine similarity = 1 - (embedding <=> query).
//! We build deterministic embeddings with known similarity values by constructing
//! orthogonal unit vectors and linear combinations of them.

mod common;
use common::builders::MemoryBuilder;

use std::sync::Arc;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;
use pgvector::Vector;
use uuid::Uuid;

/// Format embedding as postgres vector literal: '[0.1,0.2,...]'
fn emb_str(emb: &[f32]) -> String {
    format!("[{}]", emb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","))
}

/// Normalize a vector in-place.
fn normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Insert a mock embedding row for a memory and mark it embedding_status='complete'.
async fn insert_mock_embedding(pool: &PgPool, memory_id: &str, embedding: &[f32]) {
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at) \
         VALUES ($1, $2, $3::vector, 'test-mock', '1', $4, true, $5, $5)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(memory_id)
    .bind(emb_str(embedding))
    .bind(embedding.len() as i32)
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

/// Build a 4D embedding with a given cosine similarity to the query [1,0,0,0].
///
/// Similarity s: the resulting vector is s*e0 + sqrt(1-s^2)*e1 (normalized),
/// giving cosine similarity = s with the query.
fn embedding_with_sim(sim: f32) -> Vec<f32> {
    // Query is [1,0,0,0]. We build v = [sim, sqrt(1-sim^2), 0, 0] which is already unit.
    let sim = sim.clamp(-1.0, 1.0);
    let perp = (1.0 - sim * sim).max(0.0).sqrt();
    let mut v = vec![sim, perp, 0.0, 0.0];
    // Ensure normalized (should be already, but guard floating-point drift).
    normalize(&mut v);
    v
}

// ---------------------------------------------------------------------------
// Test 1: Empty store returns empty vec
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_discover_empty_store(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let query_vec = Vector::from(vec![1.0f32, 0.0, 0.0, 0.0]);

    let results = store
        .discover_associations(&query_vec, 0.3, 0.7, 10, None)
        .await
        .unwrap();

    assert!(results.is_empty(), "empty store should return empty vec");
}

// ---------------------------------------------------------------------------
// Test 2: Results land in the [min_sim, max_sim] sweet spot
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_discover_sweet_spot(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Query is the unit vector [1,0,0,0].
    // We store memories with varying similarity to this query:
    //   - sim ~0.95 (near-identical — should be EXCLUDED from sweet spot)
    //   - sim ~0.60 (sweet spot — should be INCLUDED)
    //   - sim ~0.45 (sweet spot — should be INCLUDED)
    //   - sim ~0.15 (too different — should be EXCLUDED)
    let memories_sims = vec![
        ("near-identical memory", 0.95f32),
        ("somewhat related memory", 0.60),
        ("loosely related memory", 0.45),
        ("very different memory", 0.15),
    ];

    for (content, sim) in &memories_sims {
        let m = store
            .store(MemoryBuilder::new().content(content).build())
            .await
            .unwrap();
        let emb = embedding_with_sim(*sim);
        insert_mock_embedding(&pool, &m.id, &emb).await;
    }

    let query_vec = Vector::from(vec![1.0f32, 0.0, 0.0, 0.0]);
    let results = store
        .discover_associations(&query_vec, 0.3, 0.7, 10, None)
        .await
        .unwrap();

    // Should contain the two sweet-spot memories
    assert_eq!(results.len(), 2, "expected 2 sweet-spot results, got {}", results.len());

    // All results must be within [0.3, 0.7]
    for (memory, sim) in &results {
        assert!(
            *sim >= 0.3 && *sim <= 0.7,
            "similarity {} for '{}' outside sweet spot [0.3, 0.7]",
            sim, memory.content
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: Near-identical and very-different memories are excluded
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_discover_excludes_near_far(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Only store memories OUTSIDE the sweet spot.
    let outside_sims = vec![
        ("too similar", 0.90f32),
        ("way too similar", 0.85),
        ("way too different", 0.10),
        ("basically unrelated", 0.05),
    ];

    for (content, sim) in &outside_sims {
        let m = store
            .store(MemoryBuilder::new().content(content).build())
            .await
            .unwrap();
        insert_mock_embedding(&pool, &m.id, &embedding_with_sim(*sim)).await;
    }

    let query_vec = Vector::from(vec![1.0f32, 0.0, 0.0, 0.0]);
    let results = store
        .discover_associations(&query_vec, 0.3, 0.7, 10, None)
        .await
        .unwrap();

    assert!(
        results.is_empty(),
        "memories outside sweet spot should be excluded, got {} result(s)",
        results.len()
    );
}

// ---------------------------------------------------------------------------
// Test 4: Project filter — only matching project memories returned
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_discover_respects_project_filter(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Store two sweet-spot memories in different projects.
    let m_alpha = store
        .store(
            MemoryBuilder::new()
                .content("alpha project memory")
                .source("test")
                .project("alpha")
                .build()
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &m_alpha.id, &embedding_with_sim(0.5)).await;

    let m_beta = store
        .store(
            MemoryBuilder::new()
                .content("beta project memory")
                .source("test")
                .project("beta")
                .build()
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &m_beta.id, &embedding_with_sim(0.55)).await;

    let query_vec = Vector::from(vec![1.0f32, 0.0, 0.0, 0.0]);

    // With project=alpha, should only see alpha memory.
    let results = store
        .discover_associations(&query_vec, 0.3, 0.7, 10, Some("alpha"))
        .await
        .unwrap();

    assert_eq!(results.len(), 1, "project filter: expected 1 result for alpha");
    assert_eq!(results[0].0.content, "alpha project memory");
}
