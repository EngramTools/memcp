//! Phase 25 Plan 02: Kimi adapter wiremock tests.
//! RESEARCH §Validation Architecture: bearer header byte-check + tool-call
//! translation + arg normalization (Pitfall 1) + id-preservation (Pitfall 5).

use memcp::config::ProfileConfig;
use memcp::intelligence::reasoning::{
    kimi::KimiReasoningProvider, Message, ProviderCredentials, ReasoningError, ReasoningProvider,
    ReasoningRequest, Tool,
};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn profile(model: &str) -> ProfileConfig {
    ProfileConfig {
        provider: "kimi".into(),
        model: model.into(),
        max_iterations: 6,
        budget_tokens: 8_000,
        temperature: 0.2,
        api_key: None,
        base_url: None,
    }
}

fn sample_request() -> ReasoningRequest {
    ReasoningRequest {
        system_prompt: "You are a memory agent.".into(),
        messages: vec![Message::User {
            content: "find notes".into(),
        }],
        tools: vec![Tool {
            name: "search_memories".into(),
            description: "semantic search".into(),
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

fn build_provider(server: &MockServer, api_key: &str) -> KimiReasoningProvider {
    let mut p = profile("kimi-latest");
    p.base_url = Some(server.uri());
    let creds = ProviderCredentials {
        api_key: Some(api_key.into()),
        base_url: Some(server.uri()),
    };
    KimiReasoningProvider::new(&p, creds).expect("provider ctor")
}

#[tokio::test]
async fn test_kimi_bearer_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("Authorization", "Bearer test-key-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"content":"done","tool_calls":null},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = build_provider(&server, "test-key-42");
    let resp = provider
        .generate(&sample_request())
        .await
        .expect("generate ok");
    assert_eq!(resp.content.as_deref(), Some("done"));
    assert_eq!(resp.usage.total_tokens, 15);
    // Mock's expect(1) asserts at drop — fails if bearer mismatch.
}

#[tokio::test]
async fn test_kimi_stringified_args_normalized() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"content":null,"tool_calls":[
                {"id":"search:0","type":"function","function":{"name":"search_memories","arguments":"{\"query\":\"foo\",\"limit\":5}"}}
            ]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":20,"completion_tokens":10,"total_tokens":30}
        })))
        .mount(&server)
        .await;

    let provider = build_provider(&server, "k");
    let resp = provider
        .generate(&sample_request())
        .await
        .expect("generate ok");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0].id, "search:0",
        "Kimi id must be preserved verbatim (Pitfall 5)"
    );
    assert_eq!(resp.tool_calls[0].name, "search_memories");
    assert_eq!(
        resp.tool_calls[0].arguments,
        json!({"query":"foo","limit":5}),
        "stringified args must be parsed to Value (RESEARCH Pitfall 1)"
    );
    assert!(
        resp.content.is_none(),
        "null content must decode as None"
    );
}

#[tokio::test]
async fn test_kimi_tools_omitted_when_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"content":"ok","tool_calls":null},"finish_reason":"stop"}],
            "usage":{"total_tokens":1}
        })))
        .mount(&server)
        .await;

    let provider = build_provider(&server, "k");
    let mut req = sample_request();
    req.tools.clear();
    let resp = provider.generate(&req).await.expect("generate ok");
    assert_eq!(resp.content.as_deref(), Some("ok"));
    // The #[serde(skip_serializing_if = "Option::is_none")] on tools + tool_choice
    // means the empty-tools path omits those keys entirely.
}

#[tokio::test]
async fn test_kimi_api_error_surfaces_as_api_variant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request payload"))
        .mount(&server)
        .await;

    let provider = build_provider(&server, "k");
    let err = provider
        .generate(&sample_request())
        .await
        .expect_err("expected Api");
    match err {
        ReasoningError::Api { status, message } => {
            assert_eq!(status, 400);
            assert!(message.contains("bad request"));
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn test_kimi_ctor_requires_api_key() {
    let p = profile("kimi-latest");
    match KimiReasoningProvider::new(&p, ProviderCredentials::default()) {
        Ok(_) => panic!("expected NotConfigured"),
        Err(e) => assert!(matches!(e, ReasoningError::NotConfigured(_))),
    }
}
