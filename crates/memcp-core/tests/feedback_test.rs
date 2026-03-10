//! Feedback integration tests.
//!
//! Tests `apply_feedback` with "useful"/"irrelevant" signals on the PostgresMemoryStore.
//! Each test uses `#[sqlx::test]` for a fresh ephemeral database.

mod common;
use common::builders::MemoryBuilder;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Test 1: "useful" feedback increases stability
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_useful_increases_stability(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let m = store
        .store(
            MemoryBuilder::new()
                .content("Feedback useful test memory")
                .build(),
        )
        .await
        .unwrap();

    // Get initial salience (defaults: stability=1.0)
    let before = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let before_salience = before.get(&m.id).cloned().unwrap_or_default();

    // Apply useful feedback
    store.apply_feedback(&m.id, "useful").await.unwrap();

    let after = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let after_salience = after.get(&m.id).cloned().unwrap_or_default();

    assert!(
        after_salience.stability > before_salience.stability,
        "useful feedback should increase stability: before={:.4}, after={:.4}",
        before_salience.stability,
        after_salience.stability
    );
    // Stability should be multiplied by 1.5 (useful factor)
    let expected = (before_salience.stability * 1.5).min(36_500.0);
    assert!(
        (after_salience.stability - expected).abs() < 0.001,
        "expected stability {:.4}, got {:.4}",
        expected,
        after_salience.stability
    );
}

// ---------------------------------------------------------------------------
// Test 2: "irrelevant" feedback decreases stability
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_irrelevant_decreases_stability(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let m = store
        .store(
            MemoryBuilder::new()
                .content("Feedback irrelevant test memory")
                .build(),
        )
        .await
        .unwrap();

    let before = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let before_salience = before.get(&m.id).cloned().unwrap_or_default();

    store.apply_feedback(&m.id, "irrelevant").await.unwrap();

    let after = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let after_salience = after.get(&m.id).cloned().unwrap_or_default();

    assert!(
        after_salience.stability < before_salience.stability,
        "irrelevant feedback should decrease stability: before={:.4}, after={:.4}",
        before_salience.stability,
        after_salience.stability
    );
    // Stability should be multiplied by 0.2 (irrelevant sharp drop), clamped to 0.1
    let expected = (before_salience.stability * 0.2).max(0.1);
    assert!(
        (after_salience.stability - expected).abs() < 0.001,
        "expected stability {:.4}, got {:.4}",
        expected,
        after_salience.stability
    );
}

// ---------------------------------------------------------------------------
// Test 3: Invalid signal returns MemcpError::Validation
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_invalid_signal_errors(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let m = store
        .store(
            MemoryBuilder::new()
                .content("Feedback invalid signal test")
                .build(),
        )
        .await
        .unwrap();

    let result = store.apply_feedback(&m.id, "bogus").await;
    assert!(result.is_err(), "invalid signal should return an error");

    // Should be a Validation error
    match result.unwrap_err() {
        memcp::errors::MemcpError::Validation { message, .. } => {
            assert!(
                message.contains("bogus"),
                "error message should mention the invalid signal: {}",
                message
            );
        }
        other => panic!("expected MemcpError::Validation, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 4: Feedback preserves reinforcement_count (per Phase 07.5-02 decision)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_feedback_preserves_reinforcement_count(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();

    let m = store
        .store(
            MemoryBuilder::new()
                .content("Feedback count preservation test")
                .build(),
        )
        .await
        .unwrap();

    // Reinforce the memory once to set a non-zero reinforcement_count
    store.reinforce_salience(&m.id, "good").await.unwrap();

    let before = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let before_count = before
        .get(&m.id)
        .cloned()
        .unwrap_or_default()
        .reinforcement_count;
    assert_eq!(
        before_count, 1,
        "reinforcement_count should be 1 after reinforce"
    );

    // Apply feedback — should not change reinforcement_count
    store.apply_feedback(&m.id, "useful").await.unwrap();

    let after = store.get_salience_data(&[m.id.clone()]).await.unwrap();
    let after_count = after
        .get(&m.id)
        .cloned()
        .unwrap_or_default()
        .reinforcement_count;
    assert_eq!(
        after_count, before_count,
        "apply_feedback should NOT change reinforcement_count (feedback is a salience signal, not FSRS reinforcement)"
    );
}
