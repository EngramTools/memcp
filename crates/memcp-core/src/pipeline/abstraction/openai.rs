//! OpenAI-compatible abstraction provider.
//!
//! Calls /chat/completions with separate prompts for L0 abstract and L1 overview.
//! Supports any OpenAI-compatible API: OpenAI, Kimi/Moonshot, local vLLM, etc.
//! Returns the model's text response for each tier.

use async_trait::async_trait;
use serde::Deserialize;

use super::{AbstractionError, AbstractionProvider};

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

/// OpenAI-compatible abstraction provider.
///
/// Works with any API that implements the OpenAI chat completions interface:
/// OpenAI, Kimi/Moonshot (api.moonshot.cn), local vLLM, Together AI, etc.
pub struct OpenAIAbstractionProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_input_chars: usize,
    abstract_prompt_template: String,
    overview_prompt_template: String,
}

impl OpenAIAbstractionProvider {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        max_input_chars: usize,
        abstract_prompt_template: String,
        overview_prompt_template: String,
    ) -> Self {
        OpenAIAbstractionProvider {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client build"),
            base_url,
            api_key,
            model,
            max_input_chars,
            abstract_prompt_template,
            overview_prompt_template,
        }
    }

    async fn call_openai(
        &self,
        system_prompt: &str,
        user_content: &str,
        max_tokens: u32,
    ) -> Result<String, AbstractionError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_content}
            ],
            "max_tokens": max_tokens,
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

        let parsed: OpenAIResponse = response.json().await.map_err(|e| {
            AbstractionError::Generation(format!("Failed to parse OpenAI response: {}", e))
        })?;

        let result = parsed
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default()
            .trim()
            .to_string();

        if result.is_empty() {
            return Err(AbstractionError::Generation(
                "OpenAI returned empty response".to_string(),
            ));
        }

        Ok(result)
    }
}

#[async_trait]
impl AbstractionProvider for OpenAIAbstractionProvider {
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

        // L0 abstract: ~100 tokens max
        self.call_openai(&prompt, truncated, 150).await
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

        // L1 overview: ~500 tokens max
        self.call_openai(&prompt, truncated, 600).await
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
