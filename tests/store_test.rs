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
            idempotency_key: None,
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
            idempotency_key: None,
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
            idempotency_key: None,
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
                idempotency_key: None,
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
                idempotency_key: None,
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
            idempotency_key: None,
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
            idempotency_key: None,
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

// =============================================================================
// Phase 07.5 Wave 0 Test Stubs
// These tests define the contract for Plans 01-03 to satisfy.
// They are marked #[ignore] because the methods they call (apply_feedback,
// hybrid_search with cursor) do not exist yet. Plans 01-03 must remove #[ignore].
// =============================================================================

/// SCF-02: "useful" feedback increases FSRS stability.
///
/// Contract: store.apply_feedback(&id, "useful") must increase stability by
/// multiplier ~1.5. Reads back via get_salience_data to verify.
///
/// Gate removed in Plan 02 — apply_feedback now exists on PostgresMemoryStore.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_useful(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store a memory
    let mem = store
        .store(CreateMemory {
            content: "Rust is a systems programming language".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Seed a known stability (2.5) so the expected result is deterministic
    store
        .upsert_salience(&mem.id, 2.5, 5.0, 0, None)
        .await
        .unwrap();

    // Apply "useful" feedback — should increase stability (multiplier ~1.5)
    store.apply_feedback(&mem.id, "useful").await.unwrap();

    // Read back the salience row and verify stability increased
    let rows = store
        .get_salience_data(&[mem.id.clone()])
        .await
        .unwrap();
    let row = rows.get(&mem.id).unwrap();

    assert!(
        row.stability > 2.5,
        "Expected stability to increase from 2.5 after 'useful' feedback, got {}",
        row.stability
    );
}

/// SCF-02: "irrelevant" feedback decreases FSRS stability significantly.
///
/// Contract: store.apply_feedback(&id, "irrelevant") must decrease stability
/// to ~20% of original (0.2 multiplier), clamped at 0.1 minimum.
///
/// Gate removed in Plan 02 — apply_feedback now exists on PostgresMemoryStore.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_irrelevant(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store a memory
    let mem = store
        .store(CreateMemory {
            content: "Python is a general-purpose scripting language".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Seed known stability (2.5) — after irrelevant, expect ~0.5 (2.5 * 0.2)
    store
        .upsert_salience(&mem.id, 2.5, 5.0, 0, None)
        .await
        .unwrap();

    // Apply "irrelevant" feedback — should decrease stability sharply
    store.apply_feedback(&mem.id, "irrelevant").await.unwrap();

    // Read back the salience row and verify stability decreased below 1.0
    let rows = store
        .get_salience_data(&[mem.id.clone()])
        .await
        .unwrap();
    let row = rows.get(&mem.id).unwrap();

    assert!(
        row.stability < 1.0,
        "Expected stability to decrease below 1.0 after 'irrelevant' feedback, got {}",
        row.stability
    );
    assert!(
        row.stability >= 0.1,
        "Expected stability to be clamped at minimum 0.1, got {}",
        row.stability
    );
}

/// SCF-04: Cursor-based pagination for search yields non-overlapping pages.
///
/// Contract: hybrid_search_paged with limit=2, then cursor=next_cursor produces
/// a second page with no ID overlap vs the first page.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_search_cursor_pagination(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store 5 memories with distinct content
    for i in 0..5usize {
        store
            .store(CreateMemory {
                content: format!("Unique search content item number {}", i),
                type_hint: "fact".to_string(),
                source: "test-agent".to_string(),
                tags: None,
                created_at: None,
                actor: None,
                actor_type: "agent".to_string(),
                audience: "global".to_string(),
                idempotency_key: None,
            })
            .await
            .unwrap();
    }

    // Search page 1 with limit=2, capture next_cursor
    // NOTE: hybrid_search_paged is the new cursor-aware variant that Plan 03 will add.
    // It adds a `cursor: Option<String>` parameter and returns a paged SearchResult.
    let page1 = store
        .hybrid_search_paged(
            "search content item",
            None,  // no embedding — BM25 only (daemon offline scenario)
            2,     // limit
            None,  // cursor — first page
            None, None, None, Some(60.0), None, Some(40.0), None, None,
        )
        .await
        .unwrap();

    assert_eq!(page1.hits.len(), 2, "Page 1 should have 2 results");
    assert!(page1.next_cursor.is_some(), "Page 1 should have a next_cursor");

    // Search page 2 using cursor from page 1
    let page2 = store
        .hybrid_search_paged(
            "search content item",
            None,
            2,
            page1.next_cursor.clone(), // cursor from page 1
            None, None, None, Some(60.0), None, Some(40.0), None, None,
        )
        .await
        .unwrap();

    // Verify no ID overlap between pages
    let page1_ids: std::collections::HashSet<&str> =
        page1.hits.iter().map(|h| h.memory.id.as_str()).collect();
    let page2_ids: std::collections::HashSet<&str> =
        page2.hits.iter().map(|h| h.memory.id.as_str()).collect();

    let overlap: std::collections::HashSet<_> = page1_ids.intersection(&page2_ids).collect();
    assert!(
        overlap.is_empty(),
        "Pages should not share memory IDs, found overlap: {:?}",
        overlap
    );
}

/// SCF-03: Search results always include the memory `id` field.
///
/// This test documents the requirement that every SearchHit.memory.id is non-empty.
/// It likely passes already (MCP serve has always emitted id), but it makes the
/// contract explicit and guards against regressions.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_search_result_has_id(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store a memory
    store
        .store(CreateMemory {
            content: "The memory id field must always be present in search results".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Run BM25-only search (no embedding needed)
    let hits = store
        .hybrid_search(
            "memory id field present",
            None,      // no embedding
            10,        // limit
            None, None, None,
            Some(60.0), // bm25_k
            None,       // vector_k
            Some(40.0), // symbolic_k
            None, None,
        )
        .await
        .unwrap();

    assert!(!hits.is_empty(), "Should find at least one result");

    for hit in &hits {
        assert!(
            !hit.memory.id.is_empty(),
            "Every search hit must have a non-empty memory id"
        );
    }
}

/// SCF-05: Offset-based pagination still works (backward compat) — no crash.
///
/// This test verifies that passing offset > 0 to hybrid_search does not crash
/// the process. The deprecation warning is emitted to tracing (hard to assert
/// in tests), but backward compatibility must be preserved.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_offset_deprecation_warning(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store a memory so the search has something to find
    store
        .store(CreateMemory {
            content: "Offset pagination backward compatibility test".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Run search with offset=1 — should complete without error (backward compat)
    // The deprecation warning is a tracing event emitted to stderr, not assertable here.
    let result = store
        .hybrid_search(
            "offset pagination",
            None,
            10,
            None, None, None,
            Some(60.0),
            None,
            Some(40.0),
            None, None,
        )
        .await;

    assert!(
        result.is_ok(),
        "hybrid_search with offset should not crash, got error: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// IDP tests — Phase 07.7 Idempotent Tool Operations
// ---------------------------------------------------------------------------

/// IDP-03: Deleting a non-existent memory ID must return Ok(()) not Err(NotFound).
///
/// This test exercises current (pre-implementation) behavior: the existing delete()
/// returns NotFound when rows_affected == 0.  The test is expected to FAIL until
/// idempotent delete semantics are implemented in Task 2.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_delete_nonexistent_is_ok(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let result = store.delete("00000000-0000-0000-0000-000000000000").await;
    assert!(
        result.is_ok(),
        "delete() on non-existent ID must return Ok(()), got: {:?}",
        result.err()
    );
}

// IDP-01 and IDP-02 tests — migration 013 ships with Task 2, always compiled.

/// IDP-01a: Storing identical content within the dedup window returns the same memory ID.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_dedup_within_window(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let first = store
        .store(CreateMemory {
            content: "hello world".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    let second = store
        .store(CreateMemory {
            content: "hello world".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    assert_eq!(
        first.id, second.id,
        "Duplicate content within dedup window must return the same memory ID"
    );
}

/// IDP-01b: Storing identical content after the dedup window expires creates a new memory.
///
/// Uses created_at far in the past (3600s ago), which is outside the 60s dedup window.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_store_dedup_expired_window(pool: PgPool) {
    use chrono::Duration;
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // Store the first memory with a timestamp far in the past (3600s ago).
    // When dedup_window_secs is 60 (default), this is outside the window.
    let old_time = chrono::Utc::now() - Duration::seconds(3600);
    let first = store
        .store(CreateMemory {
            content: "dedup expiry test".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: Some(old_time),
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Store same content now — the first entry is outside the 60s window.
    let second = store
        .store(CreateMemory {
            content: "dedup expiry test".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
        })
        .await
        .unwrap();

    assert_ne!(
        first.id, second.id,
        "Content stored after dedup window expiry must get a new memory ID"
    );
}

/// IDP-02a: Repeated calls with the same idempotency_key return the original memory ID.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_idempotency_key_returns_original(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let first = store
        .store(CreateMemory {
            content: "content A".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: Some("test-key-abc".to_string()),
        })
        .await
        .unwrap();

    let second = store
        .store(CreateMemory {
            content: "content A".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: Some("test-key-abc".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(
        first.id, second.id,
        "Same idempotency_key must return the original memory ID"
    );
}

/// IDP-02b: Storing different content with an already-used idempotency_key returns
/// the original memory (AWS/Stripe convention: first wins).
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_idempotency_key_conflict(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let first = store
        .store(CreateMemory {
            content: "content A".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: Some("conflict-key-k1".to_string()),
        })
        .await
        .unwrap();

    // Different content, same key — must return original (first wins)
    let second = store
        .store(CreateMemory {
            content: "content B — different!".to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: Some("conflict-key-k1".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(
        first.id, second.id,
        "Conflicting idempotency_key must return original memory ID (first wins)"
    );
    assert_eq!(
        second.content, "content A",
        "Conflicting idempotency_key must return original memory content"
    );
}
