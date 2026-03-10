//! PROV-10: Verify that store via each path sets source correctly.
//!
//! Import path should set source to the import source (not 'default').
//! Direct store should preserve the caller-provided source.

mod common;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, MemoryStore};
use sqlx::PgPool;

/// Direct store preserves caller-provided source.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_direct_store_preserves_source(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "Direct store test".to_string(),
            type_hint: "fact".to_string(),
            source: "my-agent".to_string(),
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
        })
        .await
        .unwrap();

    assert_eq!(
        mem.source, "my-agent",
        "Direct store should preserve source"
    );

    // Verify trust_level is 0.5 (default for agent + non-special source)
    assert!(
        (mem.trust_level - 0.5).abs() < 0.01,
        "Default agent source should get trust_level=0.5"
    );
}

/// User actor_type gets higher trust.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_user_actor_type_gets_high_trust(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let mem = store
        .store(CreateMemory {
            content: "User-provided memory".to_string(),
            type_hint: "preference".to_string(),
            source: "manual-entry".to_string(),
            tags: None,
            created_at: None,
            actor: Some("alice".to_string()),
            actor_type: "user".to_string(),
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
        })
        .await
        .unwrap();

    assert_eq!(mem.source, "manual-entry");
    assert!(
        (mem.trust_level - 0.8).abs() < 0.01,
        "User actor_type should get trust_level=0.8, got {}",
        mem.trust_level
    );
}
