//! OpenAI-backed curation provider.
//!
//! Uses cloud LLM for contradiction detection within clusters
//! and intelligent merge synthesis. Follows the same HTTP pattern as
//! summarization/openai.rs. Supports any OpenAI-compatible API.

use async_trait::async_trait;

use super::{ClusterMember, CurationAction, CurationError, CurationProvider};
use super::ollama::OllamaCurationProvider; // Reuse format_cluster and parse helpers

/// OpenAI-backed curation provider for LLM-powered memory review.
pub struct OpenAICurationProvider {
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAICurationProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
        }
    }

    /// Call OpenAI-compatible chat API.
    async fn chat(&self, system: &str, user: &str) -> Result<String, CurationError> {
        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| CurationError::Llm(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(CurationError::Llm(format!(
                "OpenAI API error (status {}): {}",
                status, text
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CurationError::Llm(format!("Failed to parse OpenAI response: {}", e)))?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| CurationError::Llm("No content in OpenAI response".to_string()))
    }
}

#[async_trait]
impl CurationProvider for OpenAICurationProvider {
    async fn review_cluster(
        &self,
        cluster: &[ClusterMember],
    ) -> Result<Vec<CurationAction>, CurationError> {
        let formatted = OllamaCurationProvider::format_cluster(cluster);
        let response = self.chat(super::ollama::REVIEW_SYSTEM_PROMPT, &formatted).await?;
        Ok(OllamaCurationProvider::parse_review_response(&response, cluster))
    }

    async fn synthesize_merge(
        &self,
        sources: &[ClusterMember],
    ) -> Result<String, CurationError> {
        let formatted = OllamaCurationProvider::format_cluster(sources);
        self.chat(super::ollama::MERGE_SYSTEM_PROMPT, &formatted).await
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
