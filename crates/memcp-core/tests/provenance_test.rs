//! Integration tests for provenance trust features (PROV-02 through PROV-06).
//!
//! Each test gets an ephemeral database via `#[sqlx::test]`.

mod common;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, ListFilter, MemoryStore};
use sqlx::PgPool;

/// PROV-02: Trust inference from source/actor_type.
/// - source="cli" → 0.8
/// - actor_type="auto-store" → 0.3
/// - source="import" → 0.4
/// - default → 0.5
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_trust_inference_from_source_and_actor_type(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    // CLI source → 0.8
    let cli_mem = store
        .store(CreateMemory {
            content: "CLI memory".to_string(),
            type_hint: "fact".to_string(),
            source: "cli".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();
    assert!(
        (cli_mem.trust_level - 0.8).abs() < 0.01,
        "CLI source should infer trust_level=0.8, got {}",
        cli_mem.trust_level
    );

    // auto-store actor_type → 0.3
    let auto_mem = store
        .store(CreateMemory {
            content: "Auto-store memory".to_string(),
            type_hint: "auto".to_string(),
            source: "sidecar".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "auto-store".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();
    assert!(
        (auto_mem.trust_level - 0.3).abs() < 0.01,
        "auto-store should infer trust_level=0.3, got {}",
        auto_mem.trust_level
    );

    // Default source/actor_type → 0.5
    let default_mem = store
        .store(CreateMemory {
            content: "Default memory".to_string(),
            type_hint: "fact".to_string(),
            source: "default".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();
    assert!(
        (default_mem.trust_level - 0.5).abs() < 0.01,
        "Default should infer trust_level=0.5, got {}",
        default_mem.trust_level
    );
}

/// PROV-03: Explicit trust_level is honored (not overridden by inference).
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_explicit_trust_level_honored(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "Explicitly trusted memory".to_string(),
            type_hint: "fact".to_string(),
            source: "default".to_string(),
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
            trust_level: Some(0.9),
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();

    assert!(
        (mem.trust_level - 0.9).abs() < 0.01,
        "Explicit trust_level=0.9 should be stored, got {}",
        mem.trust_level
    );

    // Verify it round-trips through get()
    let fetched = store.get(&mem.id).await.unwrap();
    assert!(
        (fetched.trust_level - 0.9).abs() < 0.01,
        "trust_level should round-trip through get(), got {}",
        fetched.trust_level
    );
}

/// PROV-04: session_id stored and filterable via list.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_session_id_stored_and_filterable(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "Session scoped memory".to_string(),
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
            trust_level: None,
            session_id: Some("sess-123".to_string()),
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();

    // Store another without session_id
    store
        .store(CreateMemory {
            content: "No session memory".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();

    // Verify get() returns session_id
    let fetched = store.get(&mem.id).await.unwrap();
    assert_eq!(fetched.session_id.as_deref(), Some("sess-123"));

    // List with session_id filter should return only the matching memory
    let result = store
        .list(ListFilter {
            session_id: Some("sess-123".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(result.memories.len(), 1);
    assert_eq!(result.memories[0].id, mem.id);
}

/// PROV-05: agent_role stored and filterable via list.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_agent_role_stored_and_filterable(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "Coder role memory".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: Some("coder".to_string()),
            write_path: None,
        })
        .await
        .unwrap();

    // Store another with different role
    store
        .store(CreateMemory {
            content: "Reviewer role memory".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: Some("reviewer".to_string()),
            write_path: None,
        })
        .await
        .unwrap();

    // Verify get() returns agent_role
    let fetched = store.get(&mem.id).await.unwrap();
    assert_eq!(fetched.agent_role.as_deref(), Some("coder"));

    // List with agent_role filter
    let result = store
        .list(ListFilter {
            agent_role: Some("coder".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(result.memories.len(), 1);
    assert_eq!(result.memories[0].id, mem.id);
}

/// PROV-06: update_trust_level produces audit trail in metadata.trust_history.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_update_trust_level_audit_trail(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "Memory to adjust trust".to_string(),
            type_hint: "fact".to_string(),
            source: "default".to_string(),
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
            trust_level: None,
            session_id: None,
            agent_role: None,
            write_path: None,
        })
        .await
        .unwrap();

    // Initial trust should be 0.5 (default inference)
    assert!((mem.trust_level - 0.5).abs() < 0.01);

    // Update trust level with reason
    store
        .update_trust_level(&mem.id, 0.3, "suspicious_pattern")
        .await
        .unwrap();

    // Re-fetch and verify
    let updated = store.get(&mem.id).await.unwrap();
    assert!(
        (updated.trust_level - 0.3).abs() < 0.01,
        "trust_level should be 0.3 after update, got {}",
        updated.trust_level
    );

    // Verify trust_history in metadata
    let history = updated
        .metadata
        .get("trust_history")
        .expect("metadata should have trust_history");
    let entries = history
        .as_array()
        .expect("trust_history should be an array");
    assert_eq!(entries.len(), 1);

    let entry = &entries[0];
    assert!((entry["from"].as_f64().unwrap() - 0.5).abs() < 0.01);
    assert!((entry["to"].as_f64().unwrap() - 0.3).abs() < 0.01);
    assert_eq!(entry["reason"].as_str().unwrap(), "suspicious_pattern");
    assert!(entry["at"].as_str().is_some(), "should have timestamp");
}
