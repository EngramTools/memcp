/// Filter strategy trait and implementations for auto-store sidecar.
///
/// Decides whether a parsed log entry is worth storing as a memory.
/// Three modes: LLM-based (default), heuristic keyword matching, or no filtering.

use async_trait::async_trait;
use serde::Deserialize;

use crate::errors::MemcpError;
use super::parser::ParsedEntry;

/// Trait for deciding whether a parsed entry should be stored.
#[async_trait]
pub trait FilterStrategy: Send + Sync {
    /// Returns true if the entry contains information worth remembering.
    async fn should_store(&self, entry: &ParsedEntry) -> Result<bool, MemcpError>;
}

/// LLM-based filter — calls Ollama or OpenAI to decide relevance.
///
/// Sends a concise prompt asking if the text contains decisions, preferences,
/// facts, instructions, or context worth remembering. Expects YES/NO response.
pub struct LlmFilter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    provider: String,
    api_key: Option<String>,
}

impl LlmFilter {
    pub fn new(provider: String, base_url: String, model: String, api_key: Option<String>) -> Self {
        LlmFilter {
            client: reqwest::Client::new(),
            base_url,
            model,
            provider,
            api_key,
        }
    }

    fn build_prompt(content: &str) -> String {
        format!(
            "Does the following text contain a decision, preference, fact, instruction, \
             or context worth remembering long-term? Reply YES or NO only.\n\n\
             Text:\n{}",
            content
        )
    }
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

#[async_trait]
impl FilterStrategy for LlmFilter {
    async fn should_store(&self, entry: &ParsedEntry) -> Result<bool, MemcpError> {
        let prompt = Self::build_prompt(&entry.content);

        let response_text = match self.provider.as_str() {
            "openai" => {
                let api_key = self.api_key.as_deref().ok_or_else(|| {
                    MemcpError::Config("OpenAI API key required for LLM filter".to_string())
                })?;
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": [{"role": "user", "content": prompt}],
                    "max_tokens": 5,
                    "temperature": 0.0
                });
                let resp = self
                    .client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("Authorization", format!("Bearer {}", api_key))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| MemcpError::Internal(format!("LLM filter request failed: {}", e)))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(status = %status, body = %body, "LLM filter API error, defaulting to store");
                    return Ok(true);
                }

                let parsed: OpenAIResponse = resp
                    .json()
                    .await
                    .map_err(|e| MemcpError::Internal(format!("LLM filter parse error: {}", e)))?;
                parsed
                    .choices
                    .first()
                    .and_then(|c| c.message.content.clone())
                    .unwrap_or_default()
            }
            _ => {
                // Ollama
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": [{"role": "user", "content": prompt}],
                    "stream": false,
                    "options": {"temperature": 0.0, "num_predict": 5}
                });
                let resp = self
                    .client
                    .post(format!("{}/api/chat", self.base_url))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| MemcpError::Internal(format!("LLM filter request failed: {}", e)))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(status = %status, body = %body, "LLM filter API error, defaulting to store");
                    return Ok(true);
                }

                let parsed: OllamaChatResponse = resp
                    .json()
                    .await
                    .map_err(|e| MemcpError::Internal(format!("LLM filter parse error: {}", e)))?;
                parsed.message.content
            }
        };

        let answer = response_text.trim().to_uppercase();
        Ok(answer.starts_with("YES"))
    }
}

/// Heuristic keyword-based filter.
///
/// Triggers on patterns indicating decisions, preferences, conventions, or rules.
/// Also triggers on longer declarative messages (>100 chars).
pub struct HeuristicFilter;

/// Keywords/phrases that indicate memory-worthy content.
const HEURISTIC_TRIGGERS: &[&str] = &[
    "always",
    "never",
    "prefer",
    "use ",
    "remember",
    "convention",
    "rule",
    "decision",
    "chose",
    "choose",
    "default to",
    "make sure",
    "important:",
    "note:",
    "todo:",
    "don't",
    "do not",
    "must",
    "should",
    "architecture",
    "pattern",
    "standard",
    "we use",
    "we don't",
    "configured",
    "setup",
    "workflow",
];

#[async_trait]
impl FilterStrategy for HeuristicFilter {
    async fn should_store(&self, entry: &ParsedEntry) -> Result<bool, MemcpError> {
        let lower = entry.content.to_lowercase();

        // Check keyword triggers
        for trigger in HEURISTIC_TRIGGERS {
            if lower.contains(trigger) {
                return Ok(true);
            }
        }

        // Longer declarative messages are more likely to be worth storing
        if entry.content.len() > 100 {
            // Check for declarative sentence structure (contains a period or colon)
            if lower.contains('.') || lower.contains(':') {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

/// No filtering — stores every parsed entry.
pub struct NoFilter;

#[async_trait]
impl FilterStrategy for NoFilter {
    async fn should_store(&self, _entry: &ParsedEntry) -> Result<bool, MemcpError> {
        Ok(true)
    }
}

/// Create a filter strategy from config values.
pub fn create_filter(
    mode: &str,
    provider: &str,
    model: &str,
    extraction_config: &crate::config::ExtractionConfig,
) -> Box<dyn FilterStrategy> {
    match mode {
        "llm" => {
            let (base_url, api_key) = match provider {
                "openai" => (
                    "https://api.openai.com/v1".to_string(),
                    extraction_config.openai_api_key.clone(),
                ),
                _ => (
                    extraction_config.ollama_base_url.clone(),
                    None,
                ),
            };
            Box::new(LlmFilter::new(
                provider.to_string(),
                base_url,
                model.to_string(),
                api_key,
            ))
        }
        "heuristic" => Box::new(HeuristicFilter),
        "none" => Box::new(NoFilter),
        other => {
            tracing::warn!(mode = other, "Unknown filter mode, falling back to heuristic");
            Box::new(HeuristicFilter)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_entry(content: &str) -> ParsedEntry {
        ParsedEntry {
            content: content.to_string(),
            timestamp: None,
            source: "test".to_string(),
            actor: None,
            session_id: None,
            project: None,
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_no_filter_always_stores() {
        let filter = NoFilter;
        assert!(filter.should_store(&make_entry("anything")).await.unwrap());
        assert!(filter.should_store(&make_entry("")).await.unwrap());
    }

    #[tokio::test]
    async fn test_heuristic_filter_triggers() {
        let filter = HeuristicFilter;

        // Keyword triggers
        assert!(filter.should_store(&make_entry("Always use pnpm")).await.unwrap());
        assert!(filter.should_store(&make_entry("never commit without tests")).await.unwrap());
        assert!(filter.should_store(&make_entry("We prefer TypeScript")).await.unwrap());
        assert!(filter.should_store(&make_entry("Remember to run lint")).await.unwrap());

        // Short non-triggering content
        assert!(!filter.should_store(&make_entry("ok")).await.unwrap());
        assert!(!filter.should_store(&make_entry("thanks")).await.unwrap());
    }

    #[tokio::test]
    async fn test_heuristic_filter_long_declarative() {
        let filter = HeuristicFilter;
        let long_text = "The project uses a microservices architecture with gRPC for inter-service communication. Each service has its own database.";
        assert!(filter.should_store(&make_entry(long_text)).await.unwrap());
    }
}
