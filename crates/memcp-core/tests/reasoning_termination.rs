// Phase 25 Plan 06 — termination test coverage (REAS-07).
//
// 4 tests covering Terminal / MaxIterations / RepeatedToolCall / Transport(timeout).

mod common {
    pub mod reasoning_fixtures;
}
use common::reasoning_fixtures::{
    noop_ctx, tc_call_with_args, MockReasoningProvider, SlowMockProvider,
};

use std::sync::{Arc, Mutex};
use std::time::Duration;

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    memory_tools, run_agent_with_provider, run_agent_with_provider_and_timeout, AgentOutcome,
    ReasoningError, ReasoningResponse, TokenUsage, ToolCall,
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

fn tc_call() -> ToolCall {
    ToolCall {
        id: "x".into(),
        name: "search_memories".into(),
        arguments: serde_json::json!({"query": "foo"}),
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
async fn test_terminal() {
    let p = Arc::new(MockReasoningProvider::new(vec![Ok(resp(vec![], 15))], "m"));
    match run_agent_with_provider(p, "dreaming", &profile(), "sys", vec![], noop_ctx())
        .await
        .unwrap()
    {
        AgentOutcome::Terminal {
            iterations: 1,
            tokens_used: 15,
            ..
        } => {}
        other => panic!("expected Terminal 1/15, got {other:?}"),
    }
}

#[tokio::test]
async fn test_max_iter() {
    // 3 responses (max_iterations=3), all with tool_calls whose args DIFFER per
    // iteration so the repeated-call detector never fires. Result: MaxIterations.
    let c1 = tc_call_with_args("search_memories", serde_json::json!({"query": "foo"}));
    let c2 = tc_call_with_args("search_memories", serde_json::json!({"query": "bar"}));
    let c3 = tc_call_with_args("search_memories", serde_json::json!({"query": "baz"}));
    let p = Arc::new(MockReasoningProvider::new(
        vec![
            Ok(resp(vec![c1], 10)),
            Ok(resp(vec![c2], 10)),
            Ok(resp(vec![c3], 10)),
        ],
        "m",
    ));
    // Passing memory_tools() so dispatch_tool runs schema validation and returns
    // a structured ToolResult — the loop keeps going regardless.
    let out = run_agent_with_provider(
        p,
        "dreaming",
        &profile(),
        "sys",
        memory_tools(),
        noop_ctx(),
    )
    .await
    .unwrap();
    match out {
        AgentOutcome::MaxIterations { iterations: 3, .. } => {}
        AgentOutcome::RepeatedToolCall { .. } => {
            panic!("repeated-call detector fired unexpectedly — args must differ per iteration")
        }
        other => panic!("expected MaxIterations, got {other:?}"),
    }
}

#[tokio::test]
async fn test_repeated() {
    let call = tc_call();
    let p = Arc::new(MockReasoningProvider::new(
        vec![
            Ok(resp(vec![call.clone()], 10)),
            Ok(resp(vec![call.clone()], 10)),
            Ok(resp(vec![call.clone()], 10)),
        ],
        "m",
    ));
    // Bump max_iterations so we hit the repeated-call detector (fires on iter 3)
    // before MaxIterations would otherwise win.
    let mut prof = profile();
    prof.max_iterations = 10;
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
    assert!(
        matches!(out, AgentOutcome::RepeatedToolCall { .. }),
        "expected RepeatedToolCall after 3x identical args, got {out:?}"
    );
}

#[tokio::test]
async fn test_timeout() {
    let slow = Arc::new(SlowMockProvider {
        delay: Duration::from_millis(200),
        response: Arc::new(Mutex::new(Some(Ok(resp(vec![], 10))))),
    });
    let err = run_agent_with_provider_and_timeout(
        slow,
        "dreaming",
        &profile(),
        "sys",
        vec![],
        noop_ctx(),
        Some(Duration::from_millis(50)),
    )
    .await
    .expect_err("should timeout");
    assert!(
        matches!(err, ReasoningError::Transport(ref msg) if msg.contains("timeout")),
        "expected Transport timeout, got {err:?}"
    );
}
