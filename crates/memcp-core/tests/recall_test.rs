//! RecallEngine integration tests.
//!
//! Each test uses `#[sqlx::test(migrator = "memcp::MIGRATOR")]` for a fresh,
//! isolated ephemeral database. Mock embeddings are inserted directly via SQL —
//! no fastembed dependency required.

mod common;
use common::builders::MemoryBuilder;

use memcp::config::RecallConfig;
use memcp::recall::RecallEngine;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;
use std::sync::Arc;

/// Generate a deterministic mock embedding of length `dim`.
/// The pattern distributes a "spike" at position `seed % dim` so embeddings
/// with different seeds have different dot products with the query.
fn mock_embedding(seed: usize, dim: usize) -> Vec<f32> {
    let mut v = vec![0.001f32; dim];
    // Spike at seed position to make cosine similarity deterministic
    v[seed % dim] = 1.0;
    // Normalize so cosine similarity is well-defined
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// Format embedding as postgres vector literal: '[0.1,0.2,...]'
fn emb_str(emb: &[f32]) -> String {
    format!(
        "[{}]",
        emb.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Insert a mock embedding row into memory_embeddings and mark the memory as 'complete'.
async fn insert_mock_embedding(pool: &PgPool, memory_id: &str, embedding: &[f32]) {
    use uuid::Uuid;
    let now = chrono::Utc::now();
    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at)
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

// ---------------------------------------------------------------------------
// Test 1: Basic recall returns memories above min_relevance
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_basic(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Use dim=4 for test embeddings — pgvector handles any dimension
    // Query embedding: spike at position 0
    let query_emb = mock_embedding(0, 4);

    // Store 3 memories with type_hint=fact (required for no-extraction tier)
    for i in 0..3 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Memory {}", i))
                    .type_hint("fact")
                    .build(),
            )
            .await
            .unwrap();
        // All use same embedding as query → relevance = 1.0 (above any threshold)
        insert_mock_embedding(&pool, &m.id, &query_emb).await;
    }

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5, // Low threshold to catch all results
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-basic".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(result.count > 0, "should recall at least one memory");
    assert_eq!(result.session_id, "session-basic");
    for mem in &result.memories {
        assert!(
            mem.relevance >= 0.5,
            "all recalled memories should be above min_relevance"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: Session dedup — same session does not re-recall already-seen memories
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_session_dedup(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    // Store 2 memories
    for i in 0..2 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Session dedup memory {}", i))
                    .type_hint("fact")
                    .build(),
            )
            .await
            .unwrap();
        insert_mock_embedding(&pool, &m.id, &query_emb).await;
    }

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);

    // First recall — should return both memories
    let result1 = engine
        .recall(
            &query_emb,
            Some("session-dedup".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();
    assert!(result1.count > 0, "first recall should return memories");

    // Second recall with same session — all memories already seen, should return 0
    let result2 = engine
        .recall(
            &query_emb,
            Some("session-dedup".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        result2.count, 0,
        "second recall with same session should return 0 (already seen)"
    );

    // Recall with different session — should see memories again
    let result3 = engine
        .recall(
            &query_emb,
            Some("session-other".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();
    assert!(
        result3.count > 0,
        "different session should recall memories"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Reset clears session dedup history
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_reset_clears_session(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    let m = store
        .store(
            MemoryBuilder::new()
                .content("Memory for reset test")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &m.id, &query_emb).await;

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);

    // First recall populates session history
    let result1 = engine
        .recall(
            &query_emb,
            Some("session-reset".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();
    assert!(result1.count > 0);

    // Without reset: should return 0 (already seen)
    let result2 = engine
        .recall(
            &query_emb,
            Some("session-reset".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        result2.count, 0,
        "without reset, second recall should return 0"
    );

    // With reset=true: clears history, should return memories again
    let result3 = engine
        .recall(
            &query_emb,
            Some("session-reset".to_string()),
            true,
            None,
            &[],
        )
        .await
        .unwrap();
    assert!(
        result3.count > 0,
        "recall with reset=true should return memories again"
    );
}

// ---------------------------------------------------------------------------
// Test 4: max_memories cap is respected
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_max_memories_cap(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    // Store 10 memories with fact type (will all match)
    for i in 0..10 {
        let m = store
            .store(
                MemoryBuilder::new()
                    .content(&format!("Memory {} for cap test", i))
                    .type_hint("fact")
                    .build(),
            )
            .await
            .unwrap();
        insert_mock_embedding(&pool, &m.id, &query_emb).await;
    }

    let config = RecallConfig {
        max_memories: 3,
        min_relevance: 0.0, // Accept everything
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-cap".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(
        result.count <= 3,
        "recall should return at most max_memories=3 results, got {}",
        result.count
    );
}

// ---------------------------------------------------------------------------
// Test 5: Extraction tier — queries against extracted_facts
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_extraction_tier(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    // Store a memory with extracted_facts populated
    let m = store
        .store(
            MemoryBuilder::new()
                .content("Some unrelated base content")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &m.id, &query_emb).await;

    // Set extracted_facts manually
    sqlx::query(
        "UPDATE memories SET extracted_facts = $1::jsonb, extraction_status = 'complete' WHERE id = $2",
    )
    .bind(r#"["The user prefers dark mode"]"#)
    .bind(&m.id)
    .execute(&pool)
    .await
    .unwrap();

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5,
        ..Default::default()
    };
    // extraction_enabled=true → uses extracted_facts tier
    let engine = RecallEngine::new(Arc::clone(&store), config, true);
    let result = engine
        .recall(
            &query_emb,
            Some("session-extraction".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(
        result.count > 0,
        "extraction tier should return memories with extracted_facts"
    );
    // Content returned should be the extracted fact, not the base content
    if let Some(mem) = result.memories.first() {
        assert_eq!(
            mem.content, "The user prefers dark mode",
            "extraction tier should return fact content, not base content"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 6: No-extraction tier — only fact/summary type_hint memories
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_no_extraction_tier(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    // Store a fact memory (should be recalled)
    let fact = store
        .store(
            MemoryBuilder::new()
                .content("Fact: user prefers vim keybindings")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &fact.id, &query_emb).await;

    // Store an instruction memory (should NOT be recalled without extraction)
    let instruction = store
        .store(
            MemoryBuilder::new()
                .content("Instruction: always use dark mode")
                .type_hint("instruction")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &instruction.id, &query_emb).await;

    // Store a summary memory (should be recalled)
    let summary = store
        .store(
            MemoryBuilder::new()
                .content("Summary: user is a Rust developer")
                .type_hint("summary")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &summary.id, &query_emb).await;

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.5,
        ..Default::default()
    };
    // extraction_enabled=false → filter to fact/summary type_hint
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-noextract".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    let ids: Vec<&str> = result
        .memories
        .iter()
        .map(|m| m.memory_id.as_str())
        .collect();
    assert!(ids.contains(&fact.id.as_str()), "fact should be recalled");
    assert!(
        ids.contains(&summary.id.as_str()),
        "summary should be recalled"
    );
    assert!(
        !ids.contains(&instruction.id.as_str()),
        "instruction type should NOT be recalled without extraction"
    );
}

// ---------------------------------------------------------------------------
// Test 7: TWR-RECALL-01 — Query-based recall weights by trust_level
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_trust_weight_query_based(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    // Store two memories with identical embeddings (same raw relevance)
    let high_trust = store
        .store(
            MemoryBuilder::new()
                .content("High trust memory about Rust patterns")
                .type_hint("fact")
                .trust_level(1.0)
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &high_trust.id, &query_emb).await;

    let low_trust = store
        .store(
            MemoryBuilder::new()
                .content("Low trust memory about Rust patterns")
                .type_hint("fact")
                .trust_level(0.2)
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &low_trust.id, &query_emb).await;

    // Set trust levels via direct SQL (in case store doesn't propagate trust_level from CreateMemory)
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(1.0f32)
        .bind(&high_trust.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(0.2f32)
        .bind(&low_trust.id)
        .execute(&pool)
        .await
        .unwrap();

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.0,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-trust-query".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(result.count >= 2, "should recall both memories");

    let high_mem = result
        .memories
        .iter()
        .find(|m| m.memory_id == high_trust.id)
        .unwrap();
    let low_mem = result
        .memories
        .iter()
        .find(|m| m.memory_id == low_trust.id)
        .unwrap();

    assert!(
        high_mem.relevance > low_mem.relevance,
        "high-trust memory ({}) should have higher relevance than low-trust ({})",
        high_mem.relevance,
        low_mem.relevance
    );
    // Verify trust_level is exposed on RecalledMemory
    assert!(
        (high_mem.trust_level - 1.0).abs() < 0.01,
        "trust_level should be exposed"
    );
    assert!(
        (low_mem.trust_level - 0.2).abs() < 0.01,
        "trust_level should be exposed"
    );
}

// ---------------------------------------------------------------------------
// Test 8: TWR-RECALL-02 — Queryless recall weights by trust_level
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_trust_weight_queryless(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Store two memories with identical salience signals but different trust
    let high_trust = store
        .store(
            MemoryBuilder::new()
                .content("High trust queryless memory")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    // Insert embedding so embedding_status = 'complete'
    insert_mock_embedding(&pool, &high_trust.id, &mock_embedding(0, 4)).await;

    let low_trust = store
        .store(
            MemoryBuilder::new()
                .content("Low trust queryless memory")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &low_trust.id, &mock_embedding(0, 4)).await;

    // Set trust levels
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(1.0f32)
        .bind(&high_trust.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(0.2f32)
        .bind(&low_trust.id)
        .execute(&pool)
        .await
        .unwrap();

    // Make salience signals identical (same stability)
    sqlx::query(
        "INSERT INTO memory_salience (memory_id, stability, difficulty, reinforcement_count)
         VALUES ($1, 5.0, 5.0, 0) ON CONFLICT (memory_id) DO UPDATE SET stability = 5.0",
    )
    .bind(&high_trust.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memory_salience (memory_id, stability, difficulty, reinforcement_count)
         VALUES ($1, 5.0, 5.0, 0) ON CONFLICT (memory_id) DO UPDATE SET stability = 5.0",
    )
    .bind(&low_trust.id)
    .execute(&pool)
    .await
    .unwrap();

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.0,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall_queryless(
            Some("session-trust-queryless".to_string()),
            false,
            None,
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(result.count >= 2, "should recall both memories");

    let high_mem = result
        .memories
        .iter()
        .find(|m| m.memory_id == high_trust.id)
        .unwrap();
    let low_mem = result
        .memories
        .iter()
        .find(|m| m.memory_id == low_trust.id)
        .unwrap();

    assert!(
        high_mem.relevance > low_mem.relevance,
        "high-trust memory ({}) should rank above low-trust ({}) in queryless recall",
        high_mem.relevance,
        low_mem.relevance
    );
}

// ---------------------------------------------------------------------------
// Test 9: TWR-RECALL-03 — Trust demotion can change ranking order
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_trust_demotion_changes_ranking(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());

    // Memory A: high raw similarity but very low trust (0.1)
    let query_emb = mock_embedding(0, 4);
    let mem_a = store
        .store(
            MemoryBuilder::new()
                .content("Memory A high similarity low trust")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    // Same embedding as query → highest raw similarity
    insert_mock_embedding(&pool, &mem_a.id, &query_emb).await;
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(0.1f32)
        .bind(&mem_a.id)
        .execute(&pool)
        .await
        .unwrap();

    // Memory B: moderate similarity but full trust (1.0)
    // Use a slightly different embedding — still similar but not identical
    let mut emb_b = mock_embedding(0, 4);
    // Slightly perturb to reduce similarity
    emb_b[1] += 0.3;
    // Re-normalize
    let norm: f32 = emb_b.iter().map(|x| x * x).sum::<f32>().sqrt();
    emb_b.iter_mut().for_each(|x| *x /= norm);

    let mem_b = store
        .store(
            MemoryBuilder::new()
                .content("Memory B moderate similarity high trust")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &mem_b.id, &emb_b).await;
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(1.0f32)
        .bind(&mem_b.id)
        .execute(&pool)
        .await
        .unwrap();

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.0,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-trust-demotion".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(result.count >= 2, "should recall both memories");

    // B (trust=1.0, moderate similarity) should outrank A (trust=0.1, high similarity)
    // because A's relevance gets multiplied by 0.1 while B keeps full relevance
    let pos_a = result
        .memories
        .iter()
        .position(|m| m.memory_id == mem_a.id)
        .unwrap();
    let pos_b = result
        .memories
        .iter()
        .position(|m| m.memory_id == mem_b.id)
        .unwrap();

    assert!(
        pos_b < pos_a,
        "high-trust memory B (pos {}) should rank above low-trust memory A (pos {}) despite lower raw similarity",
        pos_b,
        pos_a
    );
}

// ---------------------------------------------------------------------------
// Test 10: TWR-RECALL-04 — Zero-trust memory still appears (floor at 0.05)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_recall_trust_floor_prevents_zero(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let query_emb = mock_embedding(0, 4);

    let zero_trust = store
        .store(
            MemoryBuilder::new()
                .content("Zero trust memory should still appear")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();
    insert_mock_embedding(&pool, &zero_trust.id, &query_emb).await;

    // Set trust to 0.0
    sqlx::query("UPDATE memories SET trust_level = $1 WHERE id = $2")
        .bind(0.0f32)
        .bind(&zero_trust.id)
        .execute(&pool)
        .await
        .unwrap();

    let config = RecallConfig {
        max_memories: 10,
        min_relevance: 0.0,
        ..Default::default()
    };
    let engine = RecallEngine::new(Arc::clone(&store), config, false);
    let result = engine
        .recall(
            &query_emb,
            Some("session-trust-floor".to_string()),
            false,
            None,
            &[],
        )
        .await
        .unwrap();

    assert!(
        result.count >= 1,
        "zero-trust memory should still appear in results"
    );

    let mem = result
        .memories
        .iter()
        .find(|m| m.memory_id == zero_trust.id)
        .unwrap();
    assert!(
        mem.relevance > 0.0,
        "zero-trust memory should have relevance > 0.0 due to floor clamping (got {})",
        mem.relevance
    );
    assert!(
        (mem.trust_level - 0.0).abs() < 0.01,
        "trust_level should be 0.0 on the RecalledMemory"
    );
}
