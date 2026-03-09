//! OpenAI-compatible summarization provider.
//!
//! Calls /chat/completions with a system prompt for summarization.
//! Supports any OpenAI-compatible API: OpenAI, Kimi/Moonshot, local vLLM, etc.
//! Returns the model's text response as the summary.

use async_trait::async_trait;
use serde::Deserialize;

use super::{SummarizationError, SummarizationProvider};

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
}

/// OpenAI-compatible summarization provider.
///
/// Works with any API that implements the OpenAI chat completions interface:
/// OpenAI, Kimi/Moonshot (api.moonshot.cn), local vLLM, Together AI, etc.
pub struct OpenAISummarizationProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_input_chars: usize,
    prompt_template: String,
}

impl OpenAISummarizationProvider {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        max_input_chars: usize,
        prompt_template: String,
    ) -> Self {
        OpenAISummarizationProvider {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model,
            max_input_chars,
            prompt_template,
        }
    }
}

#[async_trait]
impl SummarizationProvider for OpenAISummarizationProvider {
    async fn summarize(&self, content: &str) -> Result<String, SummarizationError> {
        let truncated = if content.len() > self.max_input_chars {
            tracing::debug!(
                original_len = content.len(),
                truncated_to = self.max_input_chars,
                "Content truncated for summarization"
            );
            &content[..self.max_input_chars]
        } else {
            content
        };

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": self.prompt_template},
                {"role": "user", "content": truncated}
            ],
            "max_tokens": 500,
            "temperature": 0.0
        });

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| SummarizationError::Generation(format!("HTTP request failed: {}", e)))?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(SummarizationError::Api { status, message: body });
        }

        let parsed: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| {
                SummarizationError::Generation(format!("Failed to parse OpenAI response: {}", e))
            })?;

        let summary = parsed
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default()
            .trim()
            .to_string();

        if summary.is_empty() {
            return Err(SummarizationError::Generation(
                "OpenAI returned empty summary".to_string(),
            ));
        }

        Ok(summary)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
