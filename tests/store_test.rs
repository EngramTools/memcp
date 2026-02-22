//! Store-level integration tests using `#[sqlx::test]` for ephemeral database isolation.
//!
//! Each test gets its own temporary database — created before, dropped after — so tests
//! run in parallel with zero interference. No cleanup code needed.

use std::sync::Arc;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, ListFilter, MemoryStore, UpdateMemory};
use memcp::config::Config;
use sqlx::PgPool;

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_and_get_memory(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let created = store
        .store(CreateMemory {
            content: "Rust is great".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(created.content, "Rust is great");
    assert_eq!(created.type_hint, "fact");
    assert_eq!(created.source, "test-agent");
    assert_eq!(created.access_count, 0);

    let retrieved = store.get(&created.id).await.unwrap();
    assert_eq!(retrieved.id, created.id);
    assert_eq!(retrieved.content, "Rust is great");
    assert_eq!(retrieved.type_hint, "fact");
    assert_eq!(retrieved.source, "test-agent");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_update_memory(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let created = store
        .store(CreateMemory {
            content: "Original content".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
        })
        .await
        .unwrap();

    let updated = store
        .update(
            &created.id,
            UpdateMemory {
                content: Some("Updated content".to_string()),
                tags: Some(vec!["new-tag".to_string()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.content, "Updated content");

    let retrieved = store.get(&created.id).await.unwrap();
    assert_eq!(retrieved.content, "Updated content");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_delete_memory(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let created = store
        .store(CreateMemory {
            content: "Memory to delete".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
        })
        .await
        .unwrap();

    store.delete(&created.id).await.unwrap();

    let result = store.get(&created.id).await;
    assert!(result.is_err(), "get after delete should fail");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_list_memories_with_pagination(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    for i in 0..5 {
        store
            .store(CreateMemory {
                content: format!("Memory {}", i),
                type_hint: "fact".to_string(),
                source: "test".to_string(),
                tags: None,
                created_at: None,
                actor: None,
                actor_type: "agent".to_string(),
                audience: "global".to_string(),
            })
            .await
            .unwrap();
    }

    // Page 1: limit 2
    let page1 = store
        .list(ListFilter {
            limit: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page1.memories.len(), 2);
    assert!(page1.next_cursor.is_some());

    // Page 2
    let page2 = store
        .list(ListFilter {
            limit: 2,
            cursor: page1.next_cursor,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page2.memories.len(), 2);

    // Page 3 (remaining)
    let page3 = store
        .list(ListFilter {
            limit: 2,
            cursor: page2.next_cursor,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page3.memories.len(), 1);

    // Verify no duplicates
    let all_ids: Vec<_> = page1
        .memories
        .iter()
        .chain(page2.memories.iter())
        .chain(page3.memories.iter())
        .map(|m| &m.id)
        .collect();
    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(unique.len(), 5);
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_list_memories_with_filter(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    for (content, type_hint) in [("Fact 1", "fact"), ("Fact 2", "fact"), ("Pref 1", "preference"), ("Event 1", "event")] {
        store
            .store(CreateMemory {
                content: content.to_string(),
                type_hint: type_hint.to_string(),
                source: "test".to_string(),
                tags: None,
                created_at: None,
                actor: None,
                actor_type: "agent".to_string(),
                audience: "global".to_string(),
            })
            .await
            .unwrap();
    }

    let result = store
        .list(ListFilter {
            type_hint: Some("fact".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.memories.len(), 2);
    for m in &result.memories {
        assert_eq!(m.type_hint, "fact");
    }
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_bulk_delete(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    for i in 0..3 {
        store.store(CreateMemory {
            content: format!("Temp {}", i),
            type_hint: "temporary".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
        }).await.unwrap();
    }
    for i in 0..2 {
        store.store(CreateMemory {
            content: format!("Perm {}", i),
            type_hint: "permanent".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
        }).await.unwrap();
    }

    let filter = ListFilter {
        type_hint: Some("temporary".to_string()),
        ..Default::default()
    };
    let deleted_count = store.delete_matching(&filter).await.unwrap();
    assert_eq!(deleted_count, 3);

    let remaining = store.list(ListFilter::default()).await.unwrap();
    assert_eq!(remaining.memories.len(), 2);
    for m in &remaining.memories {
        assert_eq!(m.type_hint, "permanent");
    }
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_validation_errors(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let result = store.get("00000000-0000-0000-0000-000000000000").await;
    assert!(result.is_err(), "Non-existent memory should return error");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_build_status_json_has_sidecar_fields(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();
    let (status, alive, _last_ingest, _pending, _total) =
        memcp::cli::build_status(&store, &config, false).await.unwrap();

    // Should have all top-level sections
    assert!(status.get("sidecar").is_some(), "Missing sidecar section");
    assert!(status.get("model").is_some(), "Missing model section");
    assert!(status.get("daemon").is_some(), "Missing daemon section");
    assert!(status.get("pending").is_some(), "Missing pending section");
    assert!(status.get("total_memories").is_some(), "Missing total_memories");

    // Daemon should not be alive in test context (no heartbeat written)
    assert!(!alive, "Daemon should not be alive in test context");

    // Sidecar should have expected fields
    let sidecar = status.get("sidecar").unwrap();
    assert!(sidecar.get("ingest_count_today").is_some());
    assert!(sidecar.get("watched_file_count").is_some());
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_build_status_with_check(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let config = Config::default();
    let (status, _alive, _last_ingest, _pending, _total) =
        memcp::cli::build_status(&store, &config, true).await.unwrap();

    // Should have checks section when check=true
    assert!(status.get("checks").is_some(), "Missing checks section");
    let checks = status.get("checks").unwrap();
    assert!(checks.get("database").is_some(), "Missing database check");
    assert!(checks.get("ollama").is_some(), "Missing ollama check");
    assert!(checks.get("model_cache").is_some(), "Missing model_cache check");
    assert!(checks.get("watch_paths").is_some(), "Missing watch_paths check");

    // DB should be reachable (we're running in a test with active pool)
    assert_eq!(checks.get("database").unwrap().as_bool(), Some(true));
}
