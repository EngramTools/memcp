//! Ollama summarization provider.
//!
//! Calls /api/chat with a system prompt for summarization.
//! Returns the model's text response as the summary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{SummarizationError, SummarizationProvider};

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

/// Ollama-backed summarization provider.
pub struct OllamaSummarizationProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_input_chars: usize,
    prompt_template: String,
}

impl OllamaSummarizationProvider {
    pub fn new(
        base_url: String,
        model: String,
        max_input_chars: usize,
        prompt_template: String,
    ) -> Self {
        OllamaSummarizationProvider {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client build"),
            base_url,
            model,
            max_input_chars,
            prompt_template,
        }
    }
}

#[async_trait]
impl SummarizationProvider for OllamaSummarizationProvider {
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

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: self.prompt_template.clone(),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: truncated.to_string(),
                },
            ],
            stream: false,
            options: OllamaOptions { temperature: 0.0 },
        };

        let url = format!("{}/api/chat", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| SummarizationError::Generation(format!("HTTP request failed: {}", e)))?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(SummarizationError::Api {
                status,
                message: body,
            });
        }

        let chat_response: OllamaChatResponse = response.json().await.map_err(|e| {
            SummarizationError::Generation(format!("Failed to parse Ollama response: {}", e))
        })?;

        let summary = chat_response.message.content.trim().to_string();
        if summary.is_empty() {
            return Err(SummarizationError::Generation(
                "Ollama returned empty summary".to_string(),
            ));
        }

        Ok(summary)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
