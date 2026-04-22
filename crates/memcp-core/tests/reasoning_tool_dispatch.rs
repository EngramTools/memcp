//! Phase 25 Plan 05: memory tool dispatch integration tests.
//! DB-gated: requires MEMCP_TEST_DATABASE_URL pointing at a live memcp_test DB.
//! When env var is absent, every test short-circuits to a silent no-op so
//! `cargo test` on a machine without Postgres still passes.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use memcp::intelligence::reasoning::{
    dispatch_tool, AgentCallerContext, ProviderCredentials, ToolCall,
};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, MemoryStore, StoreOutcome};
use serde_json::{json, Value};

fn sample_create(content: &str, tier: &str, source_ids: Option<Vec<String>>) -> CreateMemory {
    CreateMemory {
        content: content.into(),
        type_hint: "fact".into(),
        source: "reasoning-dispatch-test".into(),
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
        knowledge_tier: Some(tier.into()),
        source_ids,
        reply_to_id: None,
    }
}

async fn setup() -> Option<AgentCallerContext> {
    let db_url = std::env::var("MEMCP_TEST_DATABASE_URL").ok()?;
    let store = PostgresMemoryStore::new(&db_url, false).await.ok()?;
    let store_arc: Arc<dyn MemoryStore> = Arc::new(store);
    Some(AgentCallerContext {
        store: store_arc,
        creds: ProviderCredentials::default(),
        run_id: format!("test-run-{}", uuid::Uuid::new_v4()),
        final_selection: Mutex::new(HashSet::new()),
        read_but_discarded: Mutex::new(HashSet::new()),
        tombstoned: Mutex::new(HashSet::new()),
    })
}

fn tc(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("test:{name}"),
        name: name.into(),
        arguments: args,
    }
}

fn parse_body(content: &str) -> Value {
    serde_json::from_str(content).unwrap_or(Value::Null)
}

#[tokio::test]
async fn test_derived_requires_source_ids_is_error() {
    let Some(ctx) = setup().await else {
        return;
    };
    // Phase 24 D-04: derived + no source_ids should produce is_error=true with JSON body.
    let call = tc(
        "create_memory",
        json!({
            "content": "derived test content",
            "knowledge_tier": "derived"
            // source_ids missing
        }),
    );
    let result = dispatch_tool(&call, &ctx).await;
    assert!(
        result.is_error,
        "derived without source_ids must surface as is_error"
    );
    let body = parse_body(&result.content);
    assert_eq!(
        body["code"].as_str(),
        Some("storage_error"),
        "error must carry structured code field, got body: {body}"
    );
    let msg = body["error"].as_str().unwrap_or("").to_lowercase();
    assert!(
        msg.contains("source_ids") || msg.contains("derived"),
        "error message should reference source_ids/derived, got: {msg}"
    );
}

#[tokio::test]
async fn test_create_memory_rejects_unknown_knowledge_tier() {
    // HIGH #3 — unknown tier must fail schema validation, NOT leak through to storage.
    let Some(ctx) = setup().await else {
        return;
    };
    let call = tc(
        "create_memory",
        json!({
            "content": "x",
            "knowledge_tier": "episodic",  // pre-Phase-24 value — must be rejected
        }),
    );
    let result = dispatch_tool(&call, &ctx).await;
    assert!(result.is_error, "unknown knowledge_tier must be error");
    let body = parse_body(&result.content);
    assert_eq!(
        body["code"].as_str(),
        Some("schema_validation"),
        "unknown tier must be caught by schema validator (code=schema_validation), got: {body}"
    );
}

#[tokio::test]
async fn test_dispatcher_validates_args_against_schema() {
    // MEDIUM #6 — missing required field must return code=schema_validation.
    let Some(ctx) = setup().await else {
        return;
    };
    let call = tc(
        "create_memory",
        json!({
            "knowledge_tier": "explicit"
            // content missing — schema required
        }),
    );
    let result = dispatch_tool(&call, &ctx).await;
    assert!(result.is_error, "missing required 'content' must be error");
    let body = parse_body(&result.content);
    assert_eq!(body["code"].as_str(), Some("schema_validation"));
}

#[tokio::test]
async fn test_delete_blocked_on_derived_source() {
    let Some(ctx) = setup().await else {
        return;
    };
    let source_outcome = ctx
        .store
        .store_with_outcome(sample_create("source for derived", "explicit", None))
        .await
        .expect("store source");
    let source_id = source_outcome.memory().id.clone();

    let derived_call = tc(
        "create_memory",
        json!({
            "content": "a conclusion from source",
            "knowledge_tier": "derived",
            "source_ids": [source_id.clone()],
        }),
    );
    let derived_result = dispatch_tool(&derived_call, &ctx).await;
    assert!(
        !derived_result.is_error,
        "derived create should succeed: {}",
        derived_result.content
    );

    // Try to delete the source without force — blocked.
    let del_call = tc("delete_memory", json!({"id": source_id.clone()}));
    let del_result = dispatch_tool(&del_call, &ctx).await;
    assert!(
        del_result.is_error,
        "delete of live-derived-source must be blocked"
    );
    let body = parse_body(&del_result.content);
    assert_eq!(
        body["code"].as_str(),
        Some("cascade_delete_forbidden"),
        "code must be cascade_delete_forbidden, got: {body}"
    );
}

#[tokio::test]
async fn test_delete_force_if_source_bypasses_guard() {
    // HIGH #5 — with force_if_source=true, delete of a source memory succeeds and
    // ToolResult carries a warning field.
    let Some(ctx) = setup().await else {
        return;
    };
    let source_outcome = ctx
        .store
        .store_with_outcome(sample_create("source for force-delete test", "explicit", None))
        .await
        .expect("store source");
    let source_id = source_outcome.memory().id.clone();

    let derived_call = tc(
        "create_memory",
        json!({
            "content": "derived referencing source",
            "knowledge_tier": "derived",
            "source_ids": [source_id.clone()],
        }),
    );
    let dres = dispatch_tool(&derived_call, &ctx).await;
    assert!(!dres.is_error);

    // With force_if_source=true: succeed, but ToolResult includes "warning".
    let del = tc(
        "delete_memory",
        json!({"id": source_id.clone(), "force_if_source": true}),
    );
    let res = dispatch_tool(&del, &ctx).await;
    assert!(
        !res.is_error,
        "force_if_source=true must succeed: {}",
        res.content
    );
    let body: Value = serde_json::from_str(&res.content).unwrap();
    assert_eq!(body["deleted"].as_str(), Some(source_id.as_str()));
    assert!(
        body.get("warning").is_some(),
        "force_if_source=true must include warning field, got: {body}"
    );
    let warning = body["warning"].as_str().unwrap_or("");
    assert!(
        warning.to_lowercase().contains("orphan") || warning.to_lowercase().contains("force"),
        "warning text should mention orphan/force, got: {warning}"
    );
}

#[tokio::test]
async fn test_annotate_memory_appends() {
    let Some(ctx) = setup().await else {
        return;
    };
    let m = ctx
        .store
        .store_with_outcome(sample_create("annotation target", "explicit", None))
        .await
        .expect("store");
    let id = m.memory().id.clone();

    let result = dispatch_tool(
        &tc(
            "annotate_memory",
            json!({"id": id, "annotation": "note one"}),
        ),
        &ctx,
    )
    .await;
    assert!(
        !result.is_error,
        "annotate should succeed: {}",
        result.content
    );
}

#[tokio::test]
async fn test_select_final_memories_populates_context() {
    let Some(ctx) = setup().await else {
        return;
    };
    let ids = vec![
        "11111111-1111-1111-1111-111111111111".to_string(),
        "22222222-2222-2222-2222-222222222222".to_string(),
    ];
    let call = tc("select_final_memories", json!({"ids": ids.clone()}));
    let result = dispatch_tool(&call, &ctx).await;
    assert!(!result.is_error, "select_final should succeed");
    let fs = ctx.final_selection.lock().unwrap();
    assert!(fs.contains(&ids[0]));
    assert!(fs.contains(&ids[1]));
}

#[tokio::test]
async fn test_unknown_tool_is_error_with_structured_json() {
    // MEDIUM #7 — unknown-tool error MUST be structured JSON, not plain text.
    let Some(ctx) = setup().await else {
        return;
    };
    let result = dispatch_tool(&tc("totally_made_up_tool", json!({})), &ctx).await;
    assert!(result.is_error);
    let body = parse_body(&result.content);
    assert_eq!(
        body["code"].as_str(),
        Some("unknown_tool"),
        "unknown tool must have code=unknown_tool in JSON body, got: {body}"
    );
    assert!(body["error"]
        .as_str()
        .unwrap_or("")
        .contains("unknown tool"));
}

fn _suppress_drop_on_store(_s: StoreOutcome) {}
