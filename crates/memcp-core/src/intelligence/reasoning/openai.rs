//! OpenAI reasoning adapter.
//!
//! RESEARCH Pitfall 5: do NOT share wire types with kimi.rs. Copy the shape;
//! own the quirks. 25.1 adapters (DeepSeek, Qwen, MiniMax, OpenRouter) copy
//! from this module — each owns its own types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    Message, ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest,
    ReasoningResponse, Tool, ToolCall, TokenUsage,
};
use crate::config::ProfileConfig;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub struct OpenAIReasoningProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAIReasoningProvider {
    pub fn new(
        profile: &ProfileConfig,
        creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
        let api_key = creds.require_api_key("openai")?.to_string();
        let base_url = creds
            .base_url
            .clone()
            .or_else(|| profile.base_url.clone())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| ReasoningError::Transport(format!("reqwest client build: {e}")))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model: profile.model.clone(),
        })
    }
}

// ─── Wire types (OpenAI-owned) ────────────────────────────────────────

#[derive(Serialize)]
struct OaiRequest<'a> {
    model: &'a str,
    messages: Vec<OaiMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OaiTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum OaiMessage<'a> {
    System {
        content: &'a str,
    },
    User {
        content: &'a str,
    },
    Assistant {
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<OaiAssistantToolCall<'a>>>,
    },
    Tool {
        tool_call_id: &'a str,
        content: &'a str,
    },
}

#[derive(Serialize)]
struct OaiAssistantToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: OaiFunctionCallOut<'a>,
}

#[derive(Serialize)]
struct OaiFunctionCallOut<'a> {
    name: &'a str,
    arguments: String,
}

#[derive(Serialize)]
struct OaiTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OaiFunctionDef<'a>,
}

#[derive(Serialize)]
struct OaiFunctionDef<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    #[serde(default)]
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    message: OaiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OaiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OaiResponseToolCall>>,
}

#[derive(Deserialize)]
struct OaiResponseToolCall {
    /// "call_abc123" opaque UUID (OpenAI format).
    id: String,
    function: OaiFunctionCallIn,
}

#[derive(Deserialize)]
struct OaiFunctionCallIn {
    name: String,
    /// Stringified JSON — parsed at translate_out per RESEARCH Pitfall 1.
    arguments: String,
}

#[derive(Deserialize, Default)]
struct OaiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

fn translate_in<'a>(req: &'a ReasoningRequest, model: &'a str) -> OaiRequest<'a> {
    let mut messages: Vec<OaiMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OaiMessage::System {
        content: req.system_prompt.as_str(),
    });
    for m in &req.messages {
        messages.push(match m {
            Message::System { content } => OaiMessage::System {
                content: content.as_str(),
            },
            Message::User { content } => OaiMessage::User {
                content: content.as_str(),
            },
            Message::Assistant {
                content,
                tool_calls,
            } => OaiMessage::Assistant {
                content: content.as_deref(),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        tool_calls
                            .iter()
                            .map(|c| OaiAssistantToolCall {
                                id: c.id.as_str(),
                                kind: "function",
                                function: OaiFunctionCallOut {
                                    name: c.name.as_str(),
                                    arguments: serde_json::to_string(&c.arguments)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                },
                            })
                            .collect(),
                    )
                },
            },
            Message::Tool {
                tool_call_id,
                content,
            } => OaiMessage::Tool {
                tool_call_id: tool_call_id.as_str(),
                content: content.as_str(),
            },
        });
    }
    let (tools, tool_choice) = if req.tools.is_empty() {
        (None, None)
    } else {
        (
            Some(
                req.tools
                    .iter()
                    .map(|t: &Tool| OaiTool {
                        kind: "function",
                        function: OaiFunctionDef {
                            name: t.name.as_str(),
                            description: t.description.as_str(),
                            parameters: &t.parameters,
                        },
                    })
                    .collect(),
            ),
            Some("auto"),
        )
    };
    OaiRequest {
        model,
        messages,
        tools,
        tool_choice,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
    }
}

fn translate_out(resp: OaiResponse) -> Result<ReasoningResponse, ReasoningError> {
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ReasoningError::Generation("OpenAI response has no choices".into()))?;
    let OaiResponseMessage {
        content,
        tool_calls,
    } = choice.message;
    let tool_calls = tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| {
            let args_value: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .map_err(|e| {
                    ReasoningError::Generation(format!(
                        "OpenAI tool_call {} args not valid JSON: {}",
                        tc.id, e
                    ))
                })?;
            Ok(ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments: args_value,
            })
        })
        .collect::<Result<Vec<_>, ReasoningError>>()?;
    let usage = resp.usage.unwrap_or_default();
    Ok(ReasoningResponse {
        content,
        tool_calls,
        usage: TokenUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        },
        finish_reason: choice.finish_reason,
    })
}

#[async_trait]
impl ReasoningProvider for OpenAIReasoningProvider {
    #[tracing::instrument(skip(self, req), fields(model = %self.model))]
    async fn generate(
        &self,
        req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = translate_in(req, &self.model);

        let mut attempts = 0u8;
        loop {
            attempts += 1;
            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&body)
                .send()
                .await
                .map_err(|e| ReasoningError::Transport(format!("OpenAI HTTP send: {e}")))?;

            let status = resp.status().as_u16();
            if resp.status().is_success() {
                let parsed: OaiResponse = resp
                    .json()
                    .await
                    .map_err(|e| ReasoningError::Generation(format!("OpenAI body parse: {e}")))?;
                return translate_out(parsed);
            }
            let body_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            if (500..600).contains(&status) && attempts < 2 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            return Err(ReasoningError::Api {
                status,
                message: body_text,
            });
        }
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
