use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::CreateMemory;
use sqlx::PgPool;

/// Returns a zero-filled embedding vector of `dim` dimensions.
/// Use for pipeline tests where embedding content doesn't matter.
pub fn mock_embedding(dim: usize) -> Vec<f32> {
    vec![0.0f32; dim]
}

/// Returns a deterministic non-zero embedding vector for tests that need
/// distinct embeddings. Each element is derived from the seed value.
pub fn deterministic_embedding(seed: u8, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| {
            let base = seed as f32 / 255.0;
            let variation = (i as f32 * 0.001) % 0.1;
            (base + variation).min(1.0)
        })
        .collect()
}

/// Wraps `PostgresMemoryStore::from_pool(pool)` for ergonomic test setup.
pub async fn setup_store(pool: PgPool) -> PostgresMemoryStore {
    PostgresMemoryStore::from_pool(pool).await.unwrap()
}

/// Stores a memory and manually sets embedding_status='done' + inserts a
/// mock 384-dim embedding, bypassing the async embedding pipeline.
/// Returns the stored memory's ID string.
pub async fn store_and_embed(
    store: &PostgresMemoryStore,
    memory: CreateMemory,
    pool: &PgPool,
) -> String {
    use memcp::store::MemoryStore as _;
    let stored = store.store(memory).await.unwrap();
    let id = stored.id;

    // Insert a mock 384-dim embedding (zero vector)
    let embedding: Vec<f32> = vec![0.0f32; 384];
    let embedding_str = format!(
        "[{}]",
        embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );

    sqlx::query(
        "INSERT INTO memory_embeddings (memory_id, embedding, model_name, is_current)
         VALUES ($1, $2::vector, 'test-model', true)
         ON CONFLICT (memory_id) DO UPDATE
           SET embedding = EXCLUDED.embedding,
               model_name = EXCLUDED.model_name,
               is_current = true",
    )
    .bind(&id)
    .bind(&embedding_str)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE memories SET embedding_status = 'done' WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .unwrap();

    id
}
