//! Kimi (Moonshot) reasoning adapter.
//!
//! Per RESEARCH Pitfall 5: Kimi is 95% OpenAI-compatible, NOT 100%. Do NOT share
//! Request/Response struct types with openai.rs — quirks live in this file.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    Message, ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest,
    ReasoningResponse, Tool, ToolCall, TokenUsage,
};
use crate::config::ProfileConfig;

const DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";

pub struct KimiReasoningProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl KimiReasoningProvider {
    pub fn new(
        profile: &ProfileConfig,
        creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
        let api_key = creds.require_api_key("kimi")?.to_string();
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

// ─── Wire types (Kimi-owned; do NOT share with openai.rs) ──────────────

#[derive(Serialize)]
struct KimiRequest<'a> {
    model: &'a str,
    messages: Vec<KimiMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<KimiTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum KimiMessage<'a> {
    System {
        content: &'a str,
    },
    User {
        content: &'a str,
    },
    Assistant {
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<KimiAssistantToolCall<'a>>>,
    },
    Tool {
        tool_call_id: &'a str,
        content: &'a str,
    },
}

#[derive(Serialize)]
struct KimiAssistantToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: KimiFunctionCallOut<'a>,
}

#[derive(Serialize)]
struct KimiFunctionCallOut<'a> {
    name: &'a str,
    /// Outbound arguments re-stringified (Kimi expects JSON string).
    arguments: String,
}

#[derive(Serialize)]
struct KimiTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: KimiFunctionDef<'a>,
}

#[derive(Serialize)]
struct KimiFunctionDef<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Deserialize)]
struct KimiResponse {
    choices: Vec<KimiChoice>,
    #[serde(default)]
    usage: Option<KimiUsage>,
}

#[derive(Deserialize)]
struct KimiChoice {
    message: KimiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct KimiResponseMessage {
    /// Kimi sometimes emits empty string or null on tool-calling turns (RESEARCH Pitfall 5).
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<KimiResponseToolCall>>,
}

#[derive(Deserialize)]
struct KimiResponseToolCall {
    /// "search:0" format (NOT "call_xyz"). Preserve verbatim.
    id: String,
    function: KimiFunctionCallIn,
}

#[derive(Deserialize)]
struct KimiFunctionCallIn {
    name: String,
    /// Stringified JSON per OpenAI convention — normalize to parsed Value at translate_out.
    arguments: String,
}

#[derive(Deserialize, Default)]
struct KimiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

// ─── Translation ──────────────────────────────────────────────────────

fn translate_in<'a>(req: &'a ReasoningRequest, model: &'a str) -> KimiRequest<'a> {
    let mut messages: Vec<KimiMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(KimiMessage::System {
        content: req.system_prompt.as_str(),
    });
    for m in &req.messages {
        messages.push(match m {
            Message::System { content } => KimiMessage::System {
                content: content.as_str(),
            },
            Message::User { content } => KimiMessage::User {
                content: content.as_str(),
            },
            Message::Assistant {
                content,
                tool_calls,
            } => KimiMessage::Assistant {
                content: content.as_deref(),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        tool_calls
                            .iter()
                            .map(|c| KimiAssistantToolCall {
                                id: c.id.as_str(),
                                kind: "function",
                                function: KimiFunctionCallOut {
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
            } => KimiMessage::Tool {
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
                    .map(|t: &Tool| KimiTool {
                        kind: "function",
                        function: KimiFunctionDef {
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
    KimiRequest {
        model,
        messages,
        tools,
        tool_choice,
        max_tokens: req.max_tokens,
        temperature: req.temperature,
    }
}

fn translate_out(resp: KimiResponse) -> Result<ReasoningResponse, ReasoningError> {
    let choice = resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ReasoningError::Generation("Kimi response has no choices".into()))?;
    let KimiResponseMessage {
        content,
        tool_calls,
    } = choice.message;
    let tool_calls = tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| {
            // RESEARCH Pitfall 1: arguments is stringified — parse to Value.
            let args_value: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                .map_err(|e| {
                    ReasoningError::Generation(format!(
                        "Kimi tool_call {} args not valid JSON: {}",
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

// ─── Trait impl ───────────────────────────────────────────────────────

#[async_trait]
impl ReasoningProvider for KimiReasoningProvider {
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
                .map_err(|e| ReasoningError::Transport(format!("Kimi HTTP send: {e}")))?;

            let status = resp.status().as_u16();
            if resp.status().is_success() {
                let parsed: KimiResponse = resp
                    .json()
                    .await
                    .map_err(|e| ReasoningError::Generation(format!("Kimi body parse: {e}")))?;
                return translate_out(parsed);
            }
            let body_text = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            // One retry on 5xx transient errors, never on 4xx.
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
