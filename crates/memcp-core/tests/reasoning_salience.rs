//! Phase 25 Plan 07: REAS-10 salience side-effects integration tests.
//!
//! Requires `MEMCP_TEST_DATABASE_URL` pointing at a running Postgres with
//! migrations 001–029 applied. When unset, each test silently returns (0 ran)
//! via the `setup()` guard so `cargo test -p memcp-core` stays green in
//! environments without a DB.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use memcp::intelligence::reasoning::{
    apply_salience_side_effects, AgentCallerContext, ProviderCredentials,
};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, MemoryStore, StoreOutcome};

// ─── Fixtures ─────────────────────────────────────────────────────────

async fn setup() -> Option<(Arc<PostgresMemoryStore>, String)> {
    let db_url = std::env::var("MEMCP_TEST_DATABASE_URL").ok()?;
    // PostgresMemoryStore::new(url, skip_migrations). Migrations idempotent.
    let store = PostgresMemoryStore::new(&db_url, false).await.ok()?;
    let run_id = format!("test-salience-{}", uuid::Uuid::new_v4());
    Some((Arc::new(store), run_id))
}

fn sample_create(content: String) -> CreateMemory {
    // CreateMemory doesn't derive Default (deviation Rule 1 — plan said
    // `..Default::default()` but the struct has no Default impl). Construct
    // all fields explicitly, matching the pattern in reasoning_tool_dispatch.rs.
    CreateMemory {
        content,
        type_hint: "fact".into(),
        source: "reasoning-salience-test".into(),
        tags: None,
        created_at: None,
        actor: None,
        actor_type: "agent".into(),
        audience: "global".into(),
        idempotency_key: None,
        event_time: None,
        event_time_precision: None,
        project: None,
        trust_level: None,
        session_id: None,
        agent_role: None,
        write_path: Some("reasoning_agent".into()),
        knowledge_tier: Some("explicit".into()),
        source_ids: None,
        reply_to_id: None,
    }
}

async fn insert_seed(store: &PostgresMemoryStore) -> Option<String> {
    let input = sample_create(format!("salience-seed {}", uuid::Uuid::new_v4()));
    let outcome = store.store_with_outcome(input).await.ok()?;
    match outcome {
        StoreOutcome::Created(m) => Some(m.id),
        _ => None,
    }
}

async fn stability_of(store: &PostgresMemoryStore, id: &str) -> f64 {
    // memory_salience.memory_id is TEXT (not UUID — deviation Rule 1: plan
    // assumed uuid cast but migration 005 declares text). stability is REAL (f32).
    let row: (f32,) = sqlx::query_as("SELECT stability FROM memory_salience WHERE memory_id = $1")
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("salience row");
    f64::from(row.0)
}

async fn audit_count(store: &PostgresMemoryStore, run_id: &str, memory_id: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM salience_audit_log \
         WHERE run_id = $1 AND memory_id = $2::uuid",
    )
    .bind(run_id)
    .bind(memory_id)
    .fetch_one(store.pool())
    .await
    .expect("audit count");
    row.0
}

fn ctx_with(
    store: Arc<dyn MemoryStore>,
    run_id: String,
    fs: HashSet<String>,
    rbd: HashSet<String>,
    tomb: HashSet<String>,
) -> AgentCallerContext {
    AgentCallerContext {
        store,
        creds: ProviderCredentials::default(),
        run_id,
        final_selection: Mutex::new(fs),
        read_but_discarded: Mutex::new(rbd),
        tombstoned: Mutex::new(tomb),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_final_selection_boost() {
    let Some((store, run_id)) = setup().await else {
        return;
    };
    let id = insert_seed(&store).await.expect("seed");
    // Force the salience row to a known baseline (1.0) so we can assert the ×1.3.
    store
        .upsert_salience(&id, 1.0, 5.0, 0, None)
        .await
        .expect("seed salience");
    let prev = stability_of(&store, &id).await;

    let ctx = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_id.clone(),
        HashSet::from([id.clone()]),
        HashSet::new(),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx).await.expect("ok");

    let after = stability_of(&store, &id).await;
    assert!(
        (after - prev * 1.3).abs() < 0.01,
        "stability {prev} -> {after} expected ×1.3"
    );

    let audit: (String, f64) = sqlx::query_as(
        "SELECT reason, magnitude FROM salience_audit_log \
         WHERE run_id = $1 AND memory_id = $2::uuid",
    )
    .bind(&run_id)
    .bind(&id)
    .fetch_one(store.pool())
    .await
    .expect("audit row");
    assert_eq!(audit.0, "final_selection");
    assert!((audit.1 - 1.3).abs() < 0.001);
}

#[tokio::test]
async fn test_discarded_decay() {
    let Some((store, run_id)) = setup().await else {
        return;
    };
    let id = insert_seed(&store).await.expect("seed");
    store
        .upsert_salience(&id, 2.0, 5.0, 0, None)
        .await
        .expect("seed salience");
    let prev = stability_of(&store, &id).await;

    let ctx = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_id.clone(),
        HashSet::new(),
        HashSet::from([id.clone()]),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx).await.expect("ok");

    let after = stability_of(&store, &id).await;
    assert!(
        (after - prev * 0.9).abs() < 0.01,
        "{prev} -> {after} expected ×0.9"
    );

    let reason: (String,) = sqlx::query_as(
        "SELECT reason FROM salience_audit_log \
         WHERE run_id = $1 AND memory_id = $2::uuid",
    )
    .bind(&run_id)
    .bind(&id)
    .fetch_one(store.pool())
    .await
    .expect("audit");
    assert_eq!(reason.0, "discarded");
}

#[tokio::test]
async fn test_tombstone_penalty() {
    let Some((store, run_id)) = setup().await else {
        return;
    };
    let id = insert_seed(&store).await.expect("seed");
    store
        .upsert_salience(&id, 10.0, 5.0, 0, None)
        .await
        .expect("seed salience");
    let prev = stability_of(&store, &id).await;

    let ctx = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_id.clone(),
        HashSet::new(),
        HashSet::new(),
        HashSet::from([id.clone()]),
    );
    apply_salience_side_effects(&ctx).await.expect("ok");

    let after = stability_of(&store, &id).await;
    // ×0.1 with clamp floor 0.1 (T-25-07-03 — plan 00 clamp [0.1, 36500]).
    // For prev=10, result = max(1.0, 0.1) = 1.0.
    let expected = (prev * 0.1_f64).max(0.1);
    assert!(
        (after - expected).abs() < 0.01,
        "{prev} -> {after} expected {expected}"
    );

    let reason: (String,) = sqlx::query_as(
        "SELECT reason FROM salience_audit_log \
         WHERE run_id = $1 AND memory_id = $2::uuid",
    )
    .bind(&run_id)
    .bind(&id)
    .fetch_one(store.pool())
    .await
    .expect("audit");
    assert_eq!(reason.0, "tombstoned");
}

#[tokio::test]
async fn test_idempotent_via_revert() {
    // Two DIFFERENT run_ids compound (each run gets its own audit row per memory).
    // Then revert_boost on the second run_id restores prior state.
    let Some((store, _)) = setup().await else {
        return;
    };
    let id = insert_seed(&store).await.expect("seed");
    store
        .upsert_salience(&id, 1.0, 5.0, 0, None)
        .await
        .expect("seed salience");
    let prev = stability_of(&store, &id).await;

    let run_a = format!("run-a-{}", uuid::Uuid::new_v4());
    let run_b = format!("run-b-{}", uuid::Uuid::new_v4());

    let ctx_a = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_a.clone(),
        HashSet::from([id.clone()]),
        HashSet::new(),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx_a).await.expect("a");
    let after_a = stability_of(&store, &id).await;
    assert!(
        (after_a - prev * 1.3).abs() < 0.01,
        "run_a should ×1.3: {prev} -> {after_a}"
    );

    let ctx_b = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_b.clone(),
        HashSet::from([id.clone()]),
        HashSet::new(),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx_b).await.expect("b");
    let after_b = stability_of(&store, &id).await;
    assert!(
        (after_b - after_a * 1.3).abs() < 0.01,
        "two distinct runs compound: {prev} -> {after_a} -> {after_b}"
    );

    // revert run_b
    let reverted = store.revert_boost(&run_b).await.expect("revert");
    assert!(reverted >= 1, "revert_boost reported {reverted} rows");
    let after_revert = stability_of(&store, &id).await;
    assert!(
        (after_revert - after_a).abs() < 0.01,
        "after revert should equal state before run_b"
    );

    // audit rows for run_b removed
    let remaining: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM salience_audit_log WHERE run_id = $1")
            .bind(&run_b)
            .fetch_one(store.pool())
            .await
            .expect("count");
    assert_eq!(remaining.0, 0);
}

#[tokio::test]
async fn test_idempotent_double_invoke_same_run_id() {
    // Reviews HIGH #1: invoking apply_salience_side_effects twice with the SAME run_id
    // and SAME tracking set must be a no-op on the second call:
    //   - stability multiplied by 1.3 ONCE (not 1.69×)
    //   - audit row count for (run_id, memory_id) stays at 1
    //     (UNIQUE + ON CONFLICT DO NOTHING)
    let Some((store, run_id)) = setup().await else {
        return;
    };
    let id = insert_seed(&store).await.expect("seed");
    store
        .upsert_salience(&id, 1.0, 5.0, 0, None)
        .await
        .expect("seed salience");
    let prev = stability_of(&store, &id).await;

    // First invocation.
    let ctx1 = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_id.clone(),
        HashSet::from([id.clone()]),
        HashSet::new(),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx1).await.expect("first invoke");
    let after_first = stability_of(&store, &id).await;
    assert!(
        (after_first - prev * 1.3).abs() < 0.01,
        "first invoke should multiply by 1.3: {prev} -> {after_first}"
    );
    let audit_after_first = audit_count(&store, &run_id, &id).await;
    assert_eq!(
        audit_after_first, 1,
        "exactly one audit row after first invoke"
    );

    // Second invocation with IDENTICAL ctx (same run_id, same final_selection).
    // Must be a no-op.
    let ctx2 = ctx_with(
        Arc::clone(&store) as Arc<dyn MemoryStore>,
        run_id.clone(),
        HashSet::from([id.clone()]),
        HashSet::new(),
        HashSet::new(),
    );
    apply_salience_side_effects(&ctx2)
        .await
        .expect("second invoke (idempotent)");
    let after_second = stability_of(&store, &id).await;
    assert!(
        (after_second - after_first).abs() < 0.0001,
        "IDEMPOTENCY VIOLATION: second invoke changed stability from {after_first} to {after_second}. \
         Expected no-op due to UNIQUE (run_id, memory_id) + ON CONFLICT DO NOTHING."
    );
    assert!(
        (after_second - prev * 1.3).abs() < 0.01,
        "stability must still be prev * 1.3, NOT prev * 1.69 \
         (that would indicate double-boost): prev={prev} after_second={after_second}"
    );
    let audit_after_second = audit_count(&store, &run_id, &id).await;
    assert_eq!(
        audit_after_second, 1,
        "IDEMPOTENCY VIOLATION: second invoke inserted a duplicate audit row \
         (count={audit_after_second}, expected 1). UNIQUE constraint should have rejected."
    );
}
