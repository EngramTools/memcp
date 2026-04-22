//! Phase 25 Plan 04: Ollama adapter wiremock tests.
//! Covers RESEARCH Pitfall 6 capability probe + Pitfall 1 Ollama variant
//! (parsed-object arguments) + synthesized tool_call.id uniqueness.

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    ollama::OllamaReasoningProvider, Message, ProviderCredentials, ReasoningError,
    ReasoningProvider, ReasoningRequest, Tool,
};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn profile(model: &str) -> ProfileConfig {
    ProfileConfig {
        provider: "ollama".into(),
        model: model.into(),
        max_iterations: 6,
        budget_tokens: 8_000,
        temperature: 0.2,
        api_key: None,
        base_url: None,
    }
}

fn req() -> ReasoningRequest {
    ReasoningRequest {
        system_prompt: "sys".into(),
        messages: vec![Message::User {
            content: "hi".into(),
        }],
        tools: vec![Tool {
            name: "search_memories".into(),
            description: "d".into(),
            parameters: json!({"type":"object"}),
        }],
        max_tokens: 256,
        temperature: 0.2,
    }
}

fn build(server: &MockServer, model: &str) -> OllamaReasoningProvider {
    let mut p = profile(model);
    p.base_url = Some(server.uri());
    OllamaReasoningProvider::new(
        &p,
        ProviderCredentials {
            api_key: None,
            base_url: Some(server.uri()),
        },
    )
    .expect("ctor")
}

async fn mount_show_with(caps: Vec<&str>, server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": caps,
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn test_ollama_capability_probe_accepts_tools() {
    let server = MockServer::start().await;
    mount_show_with(vec!["tools", "completion"], &server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message":{"content":"done","tool_calls":null},
            "done_reason":"stop",
            "prompt_eval_count":20,"eval_count":10
        })))
        .mount(&server)
        .await;

    let p = build(&server, "qwen3:8b");
    let resp = p.generate(&req()).await.expect("ok");
    assert_eq!(resp.usage.prompt_tokens, 20);
    assert_eq!(resp.usage.completion_tokens, 10);
    assert_eq!(resp.usage.total_tokens, 30);
}

#[tokio::test]
async fn test_ollama_capability_probe_rejects_non_tool_model() {
    let server = MockServer::start().await;
    mount_show_with(vec!["completion"], &server).await;

    let p = build(&server, "gemma:7b");
    let err = p.generate(&req()).await.expect_err("should reject");
    match err {
        ReasoningError::NotConfigured(msg) => {
            assert!(msg.contains("gemma:7b"), "message must name the model");
            assert!(
                msg.contains("tool-calling") || msg.contains("tools"),
                "message must explain tool-calling gap"
            );
        }
        other => panic!("expected NotConfigured, got {other:?}"),
    }
}

#[tokio::test]
async fn test_ollama_parsed_args_not_stringified() {
    let server = MockServer::start().await;
    mount_show_with(vec!["tools"], &server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message":{"content":null,"tool_calls":[
                {"function":{"name":"search_memories","arguments":{"query":"hello","limit":3}}}
            ]},
            "done_reason":"stop",
            "prompt_eval_count":5,"eval_count":2
        })))
        .mount(&server)
        .await;

    let p = build(&server, "qwen3:8b");
    let resp = p.generate(&req()).await.expect("ok");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0].arguments,
        json!({"query":"hello","limit":3})
    );
    assert!(
        resp.tool_calls[0].arguments.is_object(),
        "Ollama args must stay as Object (Pitfall 1 Ollama variant)"
    );
}

#[tokio::test]
async fn test_ollama_synthesized_tool_call_id_format() {
    let server = MockServer::start().await;
    mount_show_with(vec!["tools"], &server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message":{"content":null,"tool_calls":[
                {"function":{"name":"search_memories","arguments":{}}},
                {"function":{"name":"search_memories","arguments":{}}}
            ]},
            "prompt_eval_count":1,"eval_count":1
        })))
        .mount(&server)
        .await;

    let p = build(&server, "qwen3:8b");
    let resp = p.generate(&req()).await.expect("ok");
    assert_eq!(resp.tool_calls.len(), 2);
    assert!(resp.tool_calls[0].id.starts_with("ollama:"));
    assert!(resp.tool_calls[1].id.starts_with("ollama:"));
    assert_ne!(
        resp.tool_calls[0].id, resp.tool_calls[1].id,
        "each call within a turn gets a distinct synthesized id"
    );
}

#[tokio::test]
async fn test_ollama_probe_cached_across_calls() {
    let server = MockServer::start().await;
    // /api/show MUST be hit exactly once — AtomicBool caches success.
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities":["tools"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message":{"content":"ok","tool_calls":null},
            "prompt_eval_count":1,"eval_count":1
        })))
        .mount(&server)
        .await;

    let p = build(&server, "qwen3:8b");
    let _ = p.generate(&req()).await.expect("first");
    let _ = p.generate(&req()).await.expect("second");
    // Drop of server enforces expect(1) on /api/show.
}
