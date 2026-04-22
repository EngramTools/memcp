// Phase 25 Plan 06 — budget test coverage (REAS-08).
//
// 3 tests: hard-stop on budget, per-turn max_tokens bounding, metric-label source guard.

mod common {
    pub mod reasoning_fixtures;
}
use common::reasoning_fixtures::{noop_ctx, MockReasoningProvider, RecordingMockProvider};

use std::sync::Arc;

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    memory_tools, run_agent_with_provider, AgentOutcome, ReasoningResponse, TokenUsage, ToolCall,
};

fn profile() -> ProfileConfig {
    ProfileConfig {
        provider: "kimi".into(),
        model: "m".into(),
        max_iterations: 3,
        budget_tokens: 10_000,
        temperature: 0.2,
        api_key: Some("x".into()),
        base_url: None,
    }
}

fn resp(tool_calls: Vec<ToolCall>, tokens: u32) -> ReasoningResponse {
    ReasoningResponse {
        content: if tool_calls.is_empty() {
            Some("done".into())
        } else {
            None
        },
        tool_calls,
        usage: TokenUsage {
            prompt_tokens: tokens / 2,
            completion_tokens: tokens / 2,
            total_tokens: tokens,
        },
        finish_reason: None,
    }
}

#[tokio::test]
async fn test_hard_stop() {
    // budget_tokens=20; 3 responses each reporting 15 tokens. Iter 0's pre-generate
    // check: 0 < 20 -> proceed; total=15 after. Iter 1's check: 15 < 20 -> proceed;
    // total=30 after. Iter 2's check: 30 >= 20 -> BudgetExceeded(tokens_used=30,
    // iterations=2). The iterations field is the pre-generate iter counter, so
    // two generate() calls completed but iter=2 did not run.
    let c = ToolCall {
        id: "x".into(),
        name: "search_memories".into(),
        arguments: serde_json::json!({"q": 1}),
    };
    let p = Arc::new(MockReasoningProvider::new(
        vec![
            Ok(resp(vec![c.clone()], 15)),
            Ok(resp(vec![c.clone()], 15)),
            Ok(resp(vec![c.clone()], 15)),
        ],
        "m",
    ));
    let mut prof = profile();
    prof.budget_tokens = 20;
    prof.max_iterations = 5;
    let out = run_agent_with_provider(
        p,
        "dreaming",
        &prof,
        "sys",
        memory_tools(),
        noop_ctx(),
    )
    .await
    .unwrap();
    match out {
        AgentOutcome::BudgetExceeded {
            tokens_used: 30,
            iterations: 2,
        } => {}
        other => panic!("expected BudgetExceeded(30, 2), got {other:?}"),
    }
}

#[tokio::test]
async fn test_max_tokens_bounded() {
    // Scenario 1: budget=100 — per-turn max_tokens should be min(100-0, 4096) = 100.
    let provider = Arc::new(RecordingMockProvider::new(vec![Ok(resp(vec![], 10))]));
    let mut prof = profile();
    prof.budget_tokens = 100;
    prof.max_iterations = 1;
    let _ = run_agent_with_provider(
        provider.clone(),
        "dreaming",
        &prof,
        "sys",
        vec![],
        noop_ctx(),
    )
    .await
    .unwrap();
    let captured = provider.captured_max_tokens.lock().unwrap().clone();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0], 100,
        "per-turn max_tokens should be the remaining budget when that's below 4096"
    );

    // Scenario 2: budget=10_000 — per-turn max_tokens should be capped at 4096.
    let prov2 = Arc::new(RecordingMockProvider::new(vec![Ok(resp(vec![], 10))]));
    let mut prof2 = profile();
    prof2.budget_tokens = 10_000;
    prof2.max_iterations = 1;
    let _ = run_agent_with_provider(
        prov2.clone(),
        "dreaming",
        &prof2,
        "sys",
        vec![],
        noop_ctx(),
    )
    .await
    .unwrap();
    assert_eq!(
        prov2.captured_max_tokens.lock().unwrap()[0],
        4096,
        "per-turn max_tokens should cap at 4096 when remaining budget is larger"
    );
}

#[test]
fn test_metric_emitted_counter_name_present_in_source() {
    // Source-level assertion: the metric's "profile" label value must be
    // `profile_name.to_string()`, NOT `profile.model.clone()`. This guards the
    // REAS-08 per-profile accounting contract — the label is the human-readable
    // profile NAME (e.g. "dreaming") so operators can attribute spend to the
    // right agent role without remapping model ids back to their owning profile.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/intelligence/reasoning/runner.rs");
    let src = std::fs::read_to_string(&path).expect("read runner");
    assert!(
        src.contains("reasoning_tokens_total"),
        "counter name reasoning_tokens_total must be emitted by runner"
    );
    assert!(
        src.contains(r#""profile" => profile_name.to_string()"#),
        r#"the "profile" label MUST be the profile NAME (profile_name.to_string()), not the model id"#
    );
    assert!(
        src.contains(r#""adapter" => profile.provider.clone()"#),
        r#"the "adapter" label MUST be profile.provider.clone()"#
    );
    // Regression guard: catches any future attempt to label the metric with
    // the model id (which drifts across profiles that share a model).
    assert!(
        !src.contains(r#""profile" => profile.model"#),
        r#"regression guard: "profile" label must NOT be profile.model"#
    );
}
