//! Phase 25 Plan 03: OpenAI adapter wiremock tests.
//! RESEARCH §Validation Architecture: bearer byte-check + Pitfall 1 stringified
//! args normalization + retry discipline (5xx once, 4xx never).

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    openai::OpenAIReasoningProvider, Message, ProviderCredentials, ReasoningError,
    ReasoningProvider, ReasoningRequest, Tool,
};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn profile(model: &str) -> ProfileConfig {
    ProfileConfig {
        provider: "openai".into(),
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
        system_prompt: "system".into(),
        messages: vec![Message::User {
            content: "hi".into(),
        }],
        tools: vec![Tool {
            name: "search_memories".into(),
            description: "desc".into(),
            parameters: json!({
                "type":"object",
                "properties":{"query":{"type":"string"}},
                "required":["query"]
            }),
        }],
        max_tokens: 256,
        temperature: 0.2,
    }
}

fn build(server: &MockServer, key: &str) -> OpenAIReasoningProvider {
    let mut p = profile("gpt-4o-mini");
    p.base_url = Some(server.uri());
    OpenAIReasoningProvider::new(
        &p,
        ProviderCredentials {
            api_key: Some(key.into()),
            base_url: Some(server.uri()),
        },
    )
    .expect("ctor")
}

#[tokio::test]
async fn test_openai_bearer_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-sk-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"content":"done","tool_calls":null},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let p = build(&server, "test-sk-1");
    let resp = p.generate(&req()).await.expect("ok");
    assert_eq!(resp.content.as_deref(), Some("done"));
    assert_eq!(resp.usage.total_tokens, 7);
}

#[tokio::test]
async fn test_openai_stringified_args_normalized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"content":null,"tool_calls":[
                {"id":"call_abc123","type":"function","function":{"name":"search_memories","arguments":"{\"query\":\"bar\"}"}}
            ]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}
        })))
        .mount(&server)
        .await;

    let p = build(&server, "k");
    let resp = p.generate(&req()).await.expect("ok");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0].id, "call_abc123",
        "OpenAI call_ id preserved verbatim"
    );
    assert_eq!(
        resp.tool_calls[0].arguments,
        json!({"query":"bar"}),
        "stringified args must be parsed to Value (Pitfall 1)"
    );
    assert!(resp.content.is_none());
}

#[tokio::test]
async fn test_openai_5xx_retries_once_then_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .expect(2) // attempt 1 + retry
        .mount(&server)
        .await;

    let p = build(&server, "k");
    let err = p.generate(&req()).await.expect_err("5xx");
    match err {
        ReasoningError::Api { status, .. } => assert_eq!(status, 503),
        other => panic!("expected Api 503, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openai_4xx_no_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .expect(1) // no retry on 4xx
        .mount(&server)
        .await;

    let p = build(&server, "k");
    let err = p.generate(&req()).await.expect_err("4xx");
    match err {
        ReasoningError::Api { status, .. } => assert_eq!(status, 401),
        other => panic!("expected Api 401, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openai_ctor_requires_api_key() {
    let p = profile("gpt-4o-mini");
    match OpenAIReasoningProvider::new(&p, ProviderCredentials::default()) {
        Ok(_) => panic!("expected NotConfigured"),
        Err(e) => assert!(matches!(e, ReasoningError::NotConfigured(_))),
    }
}
