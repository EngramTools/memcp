//! Retroactive Neighbor Enrichment — background daemon worker that enriches
//! existing memories with tags derived from their nearest neighbors.
//!
//! Purpose: New memories change the context around existing ones. A memory about
//! "Rust async patterns" stored today makes an older memory about "tokio runtime"
//! more discoverable if tagged with related concepts. This makes the memory store
//! compound over time — new information improves findability of old memories.
//!
//! The enrichment worker:
//! 1. Scans for un-enriched memories (no 'enriched' tag) in batches
//! 2. For each, finds N nearest neighbors via pgvector cosine similarity
//! 3. Calls an LLM to suggest tags based on the memory + neighbor context
//! 4. Applies suggested tags (sanitized) plus the 'enriched' provenance marker
//!
//! Tags-only enrichment — never modifies original memory content.

pub mod worker;

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

use crate::config::QueryIntelligenceConfig;

/// Errors that can occur during enrichment LLM calls.
#[derive(Debug, Error)]
pub enum EnrichmentError {
    /// LLM service is unavailable (connection refused, timeout, etc.)
    #[error("LLM unavailable: {0}")]
    LlmUnavailable(String),

    /// Provider returned an error or unparseable response
    #[error("Provider error: {0}")]
    ProviderError(String),
}

/// Result of an enrichment LLM call for a single memory.
#[derive(Debug, Clone)]
pub struct EnrichmentResult {
    /// Tags to add to the memory. Already de-duped and sanitized by the caller.
    pub tags_to_add: Vec<String>,
}

/// Trait for LLM-backed tag suggestion.
///
/// Given a memory's content and its nearest neighbors, suggests tags that
/// would improve the memory's discoverability in future search contexts.
#[async_trait]
pub trait EnrichmentProvider: Send + Sync {
    /// Suggest tags for a memory based on its content and nearest neighbors.
    ///
    /// # Arguments
    /// * `memory_content` - The full content of the memory being enriched
    /// * `neighbor_contents` - Content strings of the N nearest neighbor memories
    ///
    /// # Returns
    /// `EnrichmentResult` with 0–5 tag suggestions, or an error if the LLM call fails.
    async fn suggest_tags(
        &self,
        memory_content: &str,
        neighbor_contents: &[String],
    ) -> Result<EnrichmentResult, EnrichmentError>;
}

/// Build the prompt for LLM tag enrichment.
///
/// Instructs the LLM to analyze a memory and its nearest neighbors, identify
/// connecting themes, and suggest 1–5 short tags that would improve discoverability.
pub fn build_enrichment_prompt(memory_content: &str, neighbor_contents: &[String]) -> String {
    let neighbors_section = if neighbor_contents.is_empty() {
        "No neighbors found.".to_string()
    } else {
        neighbor_contents
            .iter()
            .enumerate()
            .map(|(i, c)| format!("Neighbor {}: {}", i + 1, c))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    format!(
        "You are helping organize an AI memory system by adding searchable tags.\n\
         \n\
         A memory and its nearest semantic neighbors are shown below. \
         Analyze the relationships between them and suggest 1–5 concise tags \
         that would make the MEMORY more findable in future searches — \
         especially when searching for topics covered by the neighbors.\n\
         \n\
         Rules for tags:\n\
         - Use lowercase alphanumeric characters, hyphens, and underscores only\n\
         - Keep tags short (1–3 words max, joined with hyphens)\n\
         - Focus on concepts, technologies, or themes NOT already obvious from the memory text\n\
         - Do NOT suggest generic tags like 'important' or 'note'\n\
         \n\
         Output only valid JSON: {{\"tags_to_add\": [\"tag1\", \"tag2\"]}}\n\
         If no useful tags can be suggested, return: {{\"tags_to_add\": []}}\n\
         \n\
         MEMORY:\n{memory_content}\n\
         \n\
         NEIGHBORS:\n{neighbors_section}"
    )
}

/// JSON schema for enrichment output (structured output for Ollama/OpenAI).
pub fn enrichment_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "tags_to_add": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": 5,
                "description": "Tags to add to the memory (lowercase, alphanumeric + hyphens/underscores)"
            }
        },
        "required": ["tags_to_add"]
    })
}

// ═══════════════════════════════════════════════════════════════════
// Ollama implementation
// ═══════════════════════════════════════════════════════════════════

/// HTTP request/response structs for Ollama /api/chat.
#[derive(serde::Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
    format: serde_json::Value,
}

#[derive(serde::Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(serde::Serialize)]
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

#[derive(Deserialize)]
struct EnrichmentOutput {
    #[serde(default)]
    tags_to_add: Vec<String>,
}

/// Ollama-backed enrichment provider.
///
/// Calls Ollama /api/chat with structured JSON output (format field).
/// Falls back gracefully on parse errors — returns empty tags rather than failing.
pub struct OllamaEnrichmentProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaEnrichmentProvider {
    pub fn new(base_url: String, model: String) -> Self {
        OllamaEnrichmentProvider {
            client: reqwest::Client::new(),
            base_url,
            model,
        }
    }

    async fn chat(&self, prompt: String) -> Result<String, EnrichmentError> {
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![OllamaMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            stream: false,
            options: OllamaOptions { temperature: 0.0 },
            format: enrichment_schema(),
        };

        let url = format!("{}/api/chat", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| EnrichmentError::LlmUnavailable(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(EnrichmentError::ProviderError(format!(
                "Ollama returned HTTP {}: {}",
                status, body
            )));
        }

        let chat_response: OllamaChatResponse = response.json().await.map_err(|e| {
            EnrichmentError::ProviderError(format!("Failed to parse Ollama response: {}", e))
        })?;

        Ok(chat_response.message.content)
    }
}

#[async_trait]
impl EnrichmentProvider for OllamaEnrichmentProvider {
    async fn suggest_tags(
        &self,
        memory_content: &str,
        neighbor_contents: &[String],
    ) -> Result<EnrichmentResult, EnrichmentError> {
        let prompt = build_enrichment_prompt(memory_content, neighbor_contents);
        let content = self.chat(prompt).await?;

        let output: EnrichmentOutput = serde_json::from_str(&content).map_err(|e| {
            EnrichmentError::ProviderError(format!(
                "Failed to parse enrichment JSON: {} (raw: {})",
                e, &content
            ))
        })?;

        Ok(EnrichmentResult {
            tags_to_add: output.tags_to_add,
        })
    }
}

/// Factory: create an enrichment provider from query intelligence config.
///
/// Reuses the QI provider configuration (Ollama/OpenAI base URL and model)
/// since enrichment uses the same LLM infrastructure.
///
/// Returns None if no QI provider is configured (fail-open: enrichment simply
/// won't run even if enabled).
pub fn create_enrichment_provider(
    qi_config: &QueryIntelligenceConfig,
) -> Option<Arc<dyn EnrichmentProvider>> {
    // Use Ollama if a base URL is configured (default). OpenAI support can be
    // added later as a separate provider — for now Ollama covers the primary path.
    let base_url = qi_config.ollama_base_url.clone();
    let model = qi_config.reranking_ollama_model.clone();

    if base_url.is_empty() {
        return None;
    }

    Some(Arc::new(OllamaEnrichmentProvider::new(base_url, model)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_enrichment_prompt_contains_memory() {
        let prompt = build_enrichment_prompt("tokio runtime internals", &[
            "Rust async patterns".to_string(),
        ]);
        assert!(prompt.contains("tokio runtime internals"));
        assert!(prompt.contains("Rust async patterns"));
        assert!(prompt.contains("tags_to_add"));
    }

    #[test]
    fn test_build_enrichment_prompt_no_neighbors() {
        let prompt = build_enrichment_prompt("some memory content", &[]);
        assert!(prompt.contains("some memory content"));
        assert!(prompt.contains("No neighbors found"));
    }

    #[test]
    fn test_enrichment_schema_structure() {
        let schema = enrichment_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["tags_to_add"].is_object());
        assert_eq!(schema["properties"]["tags_to_add"]["maxItems"], 5);
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "tags_to_add"));
    }
}
