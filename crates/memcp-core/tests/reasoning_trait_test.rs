// Phase 25 Plan 01 — trait + unified types surface test.
// GREEN: verifies every unified type is publicly reachable and that
// ReasoningProvider is object-safe (dyn dispatch compiles).

use memcp::intelligence::reasoning::{
    AgentOutcome, Message, ProviderCredentials, ReasoningError, ReasoningProvider,
    ReasoningRequest, ReasoningResponse, TokenUsage, Tool, ToolCall, ToolResult,
};

#[test]
fn trait_compiles() {
    // Object-safety: dyn ReasoningProvider is usable as a trait object.
    fn _accepts(_p: &dyn ReasoningProvider) {}

    let _req = ReasoningRequest {
        system_prompt: String::new(),
        messages: vec![],
        tools: vec![],
        max_tokens: 0,
        temperature: 0.0,
    };
    let _resp = ReasoningResponse {
        content: None,
        tool_calls: vec![],
        usage: TokenUsage::default(),
        finish_reason: None,
    };
    let _tool = Tool {
        name: "x".into(),
        description: String::new(),
        parameters: serde_json::json!({}),
    };
    let _call = ToolCall {
        id: "a".into(),
        name: "x".into(),
        arguments: serde_json::json!({}),
    };
    let _res = ToolResult {
        tool_call_id: "a".into(),
        content: String::new(),
        is_error: false,
    };
    let _msg = Message::User {
        content: "hi".into(),
    };
    let _outcome = AgentOutcome::Terminal {
        content: None,
        tokens_used: 0,
        iterations: 0,
    };
    let _creds = ProviderCredentials::default();
    let _err: ReasoningError = ReasoningError::NotConfigured("x".into());
}
