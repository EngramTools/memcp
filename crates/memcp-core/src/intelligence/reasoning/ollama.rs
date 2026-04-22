//! Ollama (local self-hosted) reasoning adapter.
//!
//! RESEARCH Pitfall 6: Ollama accepts `tools` on ANY model but only specific
//! ones honor them — silent text-only degradation is catastrophic. Probe
//! `/api/show` once per provider; refuse if `capabilities` lacks `"tools"`.
//! No Authorization header is ever sent (Ollama is unauthenticated locally).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::{
    Message, ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest,
    ReasoningResponse, Tool, ToolCall, TokenUsage,
};
use crate::config::ProfileConfig;

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub struct OllamaReasoningProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    probed_ok: AtomicBool,
    turn_counter: AtomicUsize,
}

impl OllamaReasoningProvider {
    pub fn new(
        profile: &ProfileConfig,
        creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
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
            model: profile.model.clone(),
            probed_ok: AtomicBool::new(false),
            turn_counter: AtomicUsize::new(0),
        })
    }

    /// Probe `/api/show` once per provider instance; cache success via AtomicBool.
    /// RESEARCH Pitfall 6.
    async fn ensure_capabilities(&self) -> Result<(), ReasoningError> {
        if self.probed_ok.load(Ordering::Acquire) {
            return Ok(());
        }

        let url = format!("{}/api/show", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "name": self.model }))
            .send()
            .await
            .map_err(|e| ReasoningError::Transport(format!("Ollama /api/show: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(ReasoningError::Api {
                status,
                message: body,
            });
        }
        #[derive(Deserialize)]
        struct ShowResp {
            #[serde(default)]
            capabilities: Vec<String>,
        }
        let show: ShowResp = resp
            .json()
            .await
            .map_err(|e| ReasoningError::Generation(format!("parse /api/show: {e}")))?;
        if !show.capabilities.iter().any(|c| c == "tools") {
            return Err(ReasoningError::NotConfigured(format!(
                "Ollama model '{}' lacks tool-calling. Use one of: llama3.1+, llama3.2, qwen2.5, qwen3, mistral-nemo, firefunction-v2, command-r-plus",
                self.model
            )));
        }
        self.probed_ok.store(true, Ordering::Release);
        Ok(())
    }
}

// ─── Wire types ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool<'a>>>,
    options: OllamaOptions,
}

#[derive(Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
enum OllamaMessage<'a> {
    System {
        content: &'a str,
    },
    User {
        content: &'a str,
    },
    Assistant {
        content: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<OllamaAssistantToolCall<'a>>>,
    },
    Tool {
        content: &'a str,
    },
}

#[derive(Serialize)]
struct OllamaAssistantToolCall<'a> {
    function: OllamaFunctionCallOut<'a>,
}

#[derive(Serialize)]
struct OllamaFunctionCallOut<'a> {
    name: &'a str,
    /// Ollama expects a parsed object — serialize Value verbatim, NOT stringified.
    arguments: &'a serde_json::Value,
}

#[derive(Serialize)]
struct OllamaTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: OllamaFunctionDef<'a>,
}

#[derive(Serialize)]
struct OllamaFunctionDef<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
    num_predict: i32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

#[derive(Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaFunctionCallIn,
}

#[derive(Deserialize)]
struct OllamaFunctionCallIn {
    name: String,
    /// Ollama returns a parsed object here (RESEARCH Pitfall 1 Ollama variant).
    arguments: serde_json::Value,
}

fn translate_in<'a>(req: &'a ReasoningRequest, model: &'a str) -> OllamaChatRequest<'a> {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage::System {
        content: req.system_prompt.as_str(),
    });
    for m in &req.messages {
        messages.push(match m {
            Message::System { content } => OllamaMessage::System {
                content: content.as_str(),
            },
            Message::User { content } => OllamaMessage::User {
                content: content.as_str(),
            },
            Message::Assistant {
                content,
                tool_calls,
            } => OllamaMessage::Assistant {
                content: content.as_deref(),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        tool_calls
                            .iter()
                            .map(|c| OllamaAssistantToolCall {
                                function: OllamaFunctionCallOut {
                                    name: c.name.as_str(),
                                    arguments: &c.arguments,
                                },
                            })
                            .collect(),
                    )
                },
            },
            Message::Tool { content, .. } => OllamaMessage::Tool {
                content: content.as_str(),
            },
        });
    }
    let tools = if req.tools.is_empty() {
        None
    } else {
        Some(
            req.tools
                .iter()
                .map(|t: &Tool| OllamaTool {
                    kind: "function",
                    function: OllamaFunctionDef {
                        name: t.name.as_str(),
                        description: t.description.as_str(),
                        parameters: &t.parameters,
                    },
                })
                .collect(),
        )
    };
    OllamaChatRequest {
        model,
        messages,
        stream: false,
        tools,
        options: OllamaOptions {
            temperature: req.temperature,
            num_predict: req.max_tokens as i32,
        },
    }
}

#[async_trait]
impl ReasoningProvider for OllamaReasoningProvider {
    #[tracing::instrument(skip(self, req), fields(model = %self.model))]
    async fn generate(
        &self,
        req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        self.ensure_capabilities().await?;
        let url = format!("{}/api/chat", self.base_url);
        let body = translate_in(req, &self.model);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ReasoningError::Transport(format!("Ollama /api/chat: {e}")))?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(ReasoningError::Api {
                status,
                message: msg,
            });
        }
        let parsed: OllamaChatResponse = resp
            .json()
            .await
            .map_err(|e| ReasoningError::Generation(format!("Ollama body parse: {e}")))?;

        let turn_idx = self.turn_counter.fetch_add(1, Ordering::Relaxed);
        let tool_calls = parsed
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, tc)| ToolCall {
                id: format!("ollama:{}:{}", turn_idx, i),
                name: tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect();

        Ok(ReasoningResponse {
            content: parsed.message.content,
            tool_calls,
            usage: TokenUsage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: parsed.prompt_eval_count + parsed.eval_count,
            },
            finish_reason: parsed.done_reason,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
