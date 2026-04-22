// Phase 25 Plan 06 — smoke test for the iteration-loop runner.
//
// Exercises the happy-path terminator: a single response with empty tool_calls
// exits AgentOutcome::Terminal on iteration 1.

mod common {
    pub mod reasoning_fixtures;
}
use common::reasoning_fixtures::{noop_ctx, MockReasoningProvider};

use std::sync::Arc;

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    run_agent_with_provider, AgentOutcome, ReasoningResponse, TokenUsage,
};

fn profile() -> ProfileConfig {
    ProfileConfig {
        provider: "kimi".into(),
        model: "mock".into(),
        max_iterations: 3,
        budget_tokens: 1_000,
        temperature: 0.2,
        api_key: Some("x".into()),
        base_url: None,
    }
}

#[tokio::test]
async fn loop_runner_smoke_terminal_on_empty_tool_calls() {
    let provider = Arc::new(MockReasoningProvider::new(
        vec![Ok(ReasoningResponse {
            content: Some("done".into()),
            tool_calls: vec![],
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            finish_reason: Some("stop".into()),
        })],
        "mock",
    ));

    let out = run_agent_with_provider(provider, "dreaming", &profile(), "system", vec![], noop_ctx())
        .await
        .expect("ok");
    match out {
        AgentOutcome::Terminal {
            content,
            tokens_used,
            iterations,
        } => {
            assert_eq!(content.as_deref(), Some("done"));
            assert_eq!(tokens_used, 15);
            assert_eq!(iterations, 1);
        }
        other => panic!("expected Terminal, got {other:?}"),
    }
}
