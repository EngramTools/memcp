//! Integration tests for resolve_id_prefix — prefix-based memory ID resolution.
//!
//! Each test uses `#[sqlx::test]` for ephemeral DB isolation.

mod common;
use common::builders::MemoryBuilder;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;

/// Test 1: Unique prefix resolves to the full ID.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_resolve_id_prefix_unique(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let memory = store
        .store(MemoryBuilder::new().content("unique prefix test").build())
        .await
        .unwrap();

    // Use the first 8 chars of the ID as the prefix
    let prefix = &memory.id[..8];
    let resolved = store.resolve_id_prefix(prefix).await.unwrap();
    assert_eq!(resolved, memory.id);
}

/// Test 2: Ambiguous prefix (matching 2+ IDs) returns an error containing "ambiguous".
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_resolve_id_prefix_ambiguous(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store two memories, then manually set their IDs to share a known prefix via raw SQL.
    // Since UUIDs are random, we craft two IDs with the same 4-char prefix.
    let shared_prefix = "aaaa";
    let id_a = format!("{}-{}", shared_prefix, "1111-1111-1111-111111111111");
    let id_b = format!("{}-{}", shared_prefix, "2222-2222-2222-222222222222");

    // Insert memories directly with known IDs using the pool
    sqlx::query(
        "INSERT INTO memories (id, content, type_hint, source, embedding_status, extraction_status, abstraction_status, access_count, trust_level, created_at, updated_at) \
         VALUES ($1, 'ambiguous a', 'fact', 'test', 'pending', 'pending', 'skipped', 0, 0.5, NOW(), NOW())"
    )
    .bind(&id_a)
    .execute(store.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO memories (id, content, type_hint, source, embedding_status, extraction_status, abstraction_status, access_count, trust_level, created_at, updated_at) \
         VALUES ($1, 'ambiguous b', 'fact', 'test', 'pending', 'pending', 'skipped', 0, 0.5, NOW(), NOW())"
    )
    .bind(&id_b)
    .execute(store.pool())
    .await
    .unwrap();

    let result = store.resolve_id_prefix(shared_prefix).await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("ambiguous"),
        "Expected 'ambiguous' in error, got: {}",
        err_msg
    );
}

/// Test 3: No-match prefix returns an error containing "not found".
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_resolve_id_prefix_not_found(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let result = store.resolve_id_prefix("zzz99999").await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found"),
        "Expected 'not found' in error, got: {}",
        err_msg
    );
}

/// Test 4: Full UUID passed through is returned as-is.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_resolve_id_prefix_full_uuid(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let memory = store
        .store(
            MemoryBuilder::new()
                .content("full uuid passthrough")
                .build(),
        )
        .await
        .unwrap();

    // Pass the full ID — should still resolve correctly
    let resolved = store.resolve_id_prefix(&memory.id).await.unwrap();
    assert_eq!(resolved, memory.id);
}
