//! Ollama abstraction provider.
//!
//! Calls /api/chat with separate prompts for L0 abstract and L1 overview generation.
//! Returns the model's text response for each tier.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{AbstractionError, AbstractionProvider};

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

/// Ollama-backed abstraction provider.
pub struct OllamaAbstractionProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_input_chars: usize,
    abstract_prompt_template: String,
    overview_prompt_template: String,
}

impl OllamaAbstractionProvider {
    pub fn new(
        base_url: String,
        model: String,
        max_input_chars: usize,
        abstract_prompt_template: String,
        overview_prompt_template: String,
    ) -> Self {
        OllamaAbstractionProvider {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client build"),
            base_url,
            model,
            max_input_chars,
            abstract_prompt_template,
            overview_prompt_template,
        }
    }

    async fn call_ollama(
        &self,
        system_prompt: &str,
        user_content: &str,
    ) -> Result<String, AbstractionError> {
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: user_content.to_string(),
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
            .map_err(|e| AbstractionError::Generation(format!("HTTP request failed: {}", e)))?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(AbstractionError::Api {
                status,
                message: body,
            });
        }

        let chat_response: OllamaChatResponse = response.json().await.map_err(|e| {
            AbstractionError::Generation(format!("Failed to parse Ollama response: {}", e))
        })?;

        let result = chat_response.message.content.trim().to_string();
        if result.is_empty() {
            return Err(AbstractionError::Generation(
                "Ollama returned empty response".to_string(),
            ));
        }

        Ok(result)
    }
}

#[async_trait]
impl AbstractionProvider for OllamaAbstractionProvider {
    async fn generate_abstract(&self, content: &str) -> Result<String, AbstractionError> {
        let truncated = if content.len() > self.max_input_chars {
            tracing::debug!(
                original_len = content.len(),
                truncated_to = self.max_input_chars,
                "Content truncated for abstract generation"
            );
            &content[..self.max_input_chars]
        } else {
            content
        };

        // Replace {content} placeholder in template with actual content
        let prompt = self
            .abstract_prompt_template
            .replace("{content}", truncated);

        self.call_ollama(&prompt, truncated).await
    }

    async fn generate_overview(&self, content: &str) -> Result<String, AbstractionError> {
        let truncated = if content.len() > self.max_input_chars {
            tracing::debug!(
                original_len = content.len(),
                truncated_to = self.max_input_chars,
                "Content truncated for overview generation"
            );
            &content[..self.max_input_chars]
        } else {
            content
        };

        // Replace {content} placeholder in template with actual content
        let prompt = self
            .overview_prompt_template
            .replace("{content}", truncated);

        self.call_ollama(&prompt, truncated).await
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
