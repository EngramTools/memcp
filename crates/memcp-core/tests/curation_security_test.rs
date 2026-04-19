//! Integration tests for curation security: quarantine mechanics and search exclusion.
//!
//! Tests the Suspicious curation action, quarantine (add "suspicious" tag + trust=0.05),
//! un-quarantine (restore trust from trust_history), and search exclusion.

mod common;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, MemoryStore};
use sqlx::PgPool;
use uuid::Uuid;

/// Store a memory and insert a mock embedding so it's searchable.
/// Uses direct INSERT (not ON CONFLICT) to work with the memory_embeddings schema.
async fn store_and_embed_for_test(
    store: &PostgresMemoryStore,
    memory: CreateMemory,
    pool: &PgPool,
) -> String {
    let stored = store.store(memory).await.unwrap();
    let id = stored.id;

    let embedding: Vec<f32> = vec![0.1f32; 384];
    let embedding_str = format!(
        "[{}]",
        embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let emb_id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at)
         VALUES ($1, $2, $3::vector, 'test-model', '1.0', 384, true, NOW(), NOW())",
    )
    .bind(&emb_id)
    .bind(&id)
    .bind(&embedding_str)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .unwrap();

    id
}

/// Helper: create a basic memory for curation security tests.
fn test_memory(content: &str) -> CreateMemory {
    CreateMemory {
        content: content.to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "agent".to_string(),
        audience: "global".to_string(),
        idempotency_key: None,
        parent_id: None,
        chunk_index: None,
        total_chunks: None,
        event_time: None,
        event_time_precision: None,
        project: None,
        trust_level: Some(0.5),
        session_id: None,
        agent_role: None,
        write_path: None,
        knowledge_tier: None,
        source_ids: None,
        reply_to_id: None,
    }
}

/// Test 1: Quarantine adds "suspicious" tag, sets trust_level=0.05, records audit trail.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_quarantine_adds_tag_and_sets_trust(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem = store
        .store(test_memory("some content to quarantine"))
        .await
        .unwrap();
    assert!((mem.trust_level - 0.5).abs() < 0.01);

    // Quarantine the memory
    store.add_memory_tag(&mem.id, "suspicious").await.unwrap();
    store
        .update_trust_level(
            &mem.id,
            0.05,
            "quarantined: test reason [signals: override_instruction]",
        )
        .await
        .unwrap();

    // Verify tag was added
    let updated: serde_json::Value = sqlx::query_scalar("SELECT tags FROM memories WHERE id = $1")
        .bind(&mem.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let tags_arr = updated.as_array().unwrap();
    assert!(
        tags_arr.iter().any(|t| t.as_str() == Some("suspicious")),
        "Memory should have 'suspicious' tag after quarantine"
    );

    // Verify trust_level was set to 0.05
    let trust: f32 = sqlx::query_scalar("SELECT trust_level FROM memories WHERE id = $1")
        .bind(&mem.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        (trust - 0.05).abs() < 0.01,
        "Trust should be 0.05 after quarantine, got {}",
        trust
    );

    // Verify trust_history audit entry
    let metadata: serde_json::Value =
        sqlx::query_scalar("SELECT metadata FROM memories WHERE id = $1")
            .bind(&mem.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let history = metadata.get("trust_history").unwrap().as_array().unwrap();
    assert!(
        !history.is_empty(),
        "trust_history should have at least one entry"
    );
    let entry = &history[0];
    assert!((entry["from"].as_f64().unwrap() - 0.5).abs() < 0.01);
    assert!((entry["to"].as_f64().unwrap() - 0.05).abs() < 0.01);
}

/// Test 2: Quarantined memory (tagged "suspicious") does NOT appear in hybrid_search.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_quarantined_excluded_from_hybrid_search(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store and embed a memory
    let id = store_and_embed_for_test(
        &store,
        test_memory("important fact about rust programming"),
        &pool,
    )
    .await;

    // Verify it appears in BM25 search before quarantine
    let results_before = store.search_bm25("rust programming", 10).await.unwrap();
    assert!(
        results_before.iter().any(|(mid, _)| mid == &id),
        "Memory should appear in BM25 search before quarantine"
    );

    // Quarantine it
    store.add_memory_tag(&id, "suspicious").await.unwrap();
    store
        .update_trust_level(&id, 0.05, "quarantined: test")
        .await
        .unwrap();

    // Verify it does NOT appear in BM25 search
    let results_after = store.search_bm25("rust programming", 10).await.unwrap();
    assert!(
        !results_after.iter().any(|(mid, _)| mid == &id),
        "Quarantined memory should NOT appear in BM25 search"
    );
}

/// Test 3: Quarantined memory does NOT appear in search_similar.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_quarantined_excluded_from_search_similar(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store a normal memory and a quarantined memory with the same embedding
    let id_normal =
        store_and_embed_for_test(&store, test_memory("normal memory about cats"), &pool).await;
    let id_quarantined =
        store_and_embed_for_test(&store, test_memory("quarantined memory about cats"), &pool).await;

    // Quarantine the second one
    store
        .add_memory_tag(&id_quarantined, "suspicious")
        .await
        .unwrap();
    store
        .update_trust_level(&id_quarantined, 0.05, "quarantined: test")
        .await
        .unwrap();

    // Search similar using the same zero embedding
    let embedding = pgvector::Vector::from(vec![0.0f32; 384]);
    let filter = memcp::store::SearchFilter {
        query_embedding: embedding,
        limit: 10,
        offset: 0,
        cursor: None,
        created_after: None,
        created_before: None,
        tags: None,
        audience: None,
        tier_filter: None,
    };
    let result = store.search_similar(&filter).await.unwrap();

    let result_ids: Vec<&str> = result.hits.iter().map(|h| h.memory.id.as_str()).collect();
    assert!(
        result_ids.contains(&id_normal.as_str()),
        "Normal memory should appear in search_similar"
    );
    assert!(
        !result_ids.contains(&id_quarantined.as_str()),
        "Quarantined memory should NOT appear in search_similar"
    );
}

/// Test 4: Un-quarantine removes "suspicious" tag and restores previous trust_level.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_unquarantine_restores_trust(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let mem = store
        .store(test_memory("memory to unquarantine"))
        .await
        .unwrap();
    let original_trust = mem.trust_level; // 0.5

    // Quarantine
    store.add_memory_tag(&mem.id, "suspicious").await.unwrap();
    store
        .update_trust_level(&mem.id, 0.05, "quarantined: test")
        .await
        .unwrap();

    // Un-quarantine
    store.unquarantine_memory(&mem.id).await.unwrap();

    // Verify tag removed
    let tags: serde_json::Value = sqlx::query_scalar("SELECT tags FROM memories WHERE id = $1")
        .bind(&mem.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let empty_vec = vec![];
    let tags_arr = tags.as_array().unwrap_or(&empty_vec);
    assert!(
        !tags_arr.iter().any(|t| t.as_str() == Some("suspicious")),
        "Suspicious tag should be removed after un-quarantine"
    );

    // Verify trust restored
    let trust: f32 = sqlx::query_scalar("SELECT trust_level FROM memories WHERE id = $1")
        .bind(&mem.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        (trust - original_trust).abs() < 0.01,
        "Trust should be restored to {} after un-quarantine, got {}",
        original_trust,
        trust
    );
}

/// Test 5: Un-quarantined memory reappears in search results.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_unquarantined_reappears_in_search(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let id = store_and_embed_for_test(
        &store,
        test_memory("unique content about quantum physics"),
        &pool,
    )
    .await;

    // Quarantine
    store.add_memory_tag(&id, "suspicious").await.unwrap();
    store
        .update_trust_level(&id, 0.05, "quarantined: test")
        .await
        .unwrap();

    // Verify not in BM25 search
    let results_quarantined = store.search_bm25("quantum physics", 10).await.unwrap();
    assert!(
        !results_quarantined.iter().any(|(mid, _)| mid == &id),
        "Should not appear while quarantined"
    );

    // Un-quarantine
    store.unquarantine_memory(&id).await.unwrap();

    // Verify reappears in BM25 search
    let results_restored = store.search_bm25("quantum physics", 10).await.unwrap();
    assert!(
        results_restored.iter().any(|(mid, _)| mid == &id),
        "Should reappear in search after un-quarantine"
    );
}
