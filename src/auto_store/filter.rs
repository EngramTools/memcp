/// Filter strategy trait and implementations for auto-store sidecar.
///
/// Decides whether a parsed log entry is worth storing as a memory.
/// Four modes: LLM-based, heuristic keyword matching, category-aware, or no filtering.

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Category-aware filter — blocks tool narration, passes through valuable content.
///
/// Uses compiled regex patterns to detect and reject low-value tool narration
/// (e.g. "Let me read the file...", "Now I'll edit...", "Running command...").
/// Decisions, preferences, errors, and architecture notes pass through unfiltered.
///
/// Operates purely heuristically — no LLM required. An optional `llm_fallback`
/// can be provided for ambiguous content (deferred; currently pass-through).
pub struct CategoryFilter {
    patterns: Vec<Regex>,
    llm_fallback: Option<Box<dyn FilterStrategy>>,
    filtered_count: Arc<AtomicU64>,
}

/// Default tool narration patterns compiled at construction time.
const TOOL_NARRATION_PATTERNS: &[&str] = &[
    r"(?i)^let me (read|check|look at|search|find|edit|write|run|execute|open|examine)",
    r"(?i)^now i('ll| will) (read|check|look|search|find|edit|write|run|execute)",
    r"(?i)^(reading|writing|editing|searching|checking|running|executing|looking at) (the |a )?(file|code|test|command|script|directory)",
    r"(?i)^i('ll| will) (start by|begin by|go ahead and|proceed to) ",
    r"(?i)^(here's|here is) (what i found|the output|the result)",
];

impl CategoryFilter {
    /// Create a new CategoryFilter.
    ///
    /// Compiles the default tool narration patterns plus any user-supplied patterns
    /// from `config.tool_narration_patterns`. Invalid patterns are skipped with a warning
    /// (fail-open — a bad pattern never prevents storing).
    ///
    /// An optional `llm_fallback` can be provided for ambiguous content classification
    /// (currently unused; deferred per CONTEXT.md).
    pub fn new(
        config: &crate::config::CategoryFilterConfig,
        llm_fallback: Option<Box<dyn FilterStrategy>>,
    ) -> Self {
        let mut patterns = Vec::new();

        // Compile built-in patterns
        for &pat in TOOL_NARRATION_PATTERNS {
            match Regex::new(pat) {
                Ok(re) => patterns.push(re),
                Err(e) => {
                    tracing::warn!(pattern = pat, error = %e, "CategoryFilter: built-in pattern failed to compile, skipping");
                }
            }
        }

        // Compile user-supplied patterns (fail-open on invalid regex)
        for pat in &config.tool_narration_patterns {
            match Regex::new(pat) {
                Ok(re) => patterns.push(re),
                Err(e) => {
                    tracing::warn!(pattern = %pat, error = %e, "CategoryFilter: custom pattern failed to compile, skipping");
                }
            }
        }

        CategoryFilter {
            patterns,
            llm_fallback,
            filtered_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Returns the number of entries filtered since construction.
    pub fn filtered_count(&self) -> u64 {
        self.filtered_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl FilterStrategy for CategoryFilter {
    async fn should_store(&self, entry: &ParsedEntry) -> Result<bool, MemcpError> {
        // Step 1: Check tool narration patterns
        for pattern in &self.patterns {
            if pattern.is_match(&entry.content) {
                self.filtered_count.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    content = %entry.content.chars().take(80).collect::<String>(),
                    "Filtered: tool narration"
                );
                return Ok(false);
            }
        }

        // Step 2: If LLM fallback available, could check ambiguous content here.
        // For now, pass through anything that doesn't match patterns.
        // LLM expansion is deferred per CONTEXT.md.
        let _ = &self.llm_fallback; // suppress unused warning

        Ok(true)
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
    auto_store_config: &crate::config::AutoStoreConfig,
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
        "category" => {
            Box::new(CategoryFilter::new(&auto_store_config.category_filter, None))
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

    fn make_category_filter(extra_patterns: Vec<String>) -> CategoryFilter {
        let config = crate::config::CategoryFilterConfig {
            enabled: true,
            block_tool_narration: true,
            tool_narration_patterns: extra_patterns,
        };
        CategoryFilter::new(&config, None)
    }

    #[tokio::test]
    async fn test_category_filter_blocks_narration() {
        let filter = make_category_filter(vec![]);

        // Built-in narration patterns must be blocked
        assert!(!filter.should_store(&make_entry("Let me read the file")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Let me check the code")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Now I'll edit the code")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Now I will run the test")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Reading the file src/main.rs")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Running command ls -la")).await.unwrap());
        assert!(!filter.should_store(&make_entry("I'll start by looking at the code")).await.unwrap());
        assert!(!filter.should_store(&make_entry("I will begin by reading the directory")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Here's what I found")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Here is the output")).await.unwrap());

        // Verify filtered_count is tracking correctly
        assert!(filter.filtered_count() > 0);
    }

    #[tokio::test]
    async fn test_category_filter_passes_decisions() {
        let filter = make_category_filter(vec![]);

        // Decisions, preferences, errors, and architecture notes must pass through
        assert!(filter.should_store(&make_entry("Always use pnpm for this project")).await.unwrap());
        assert!(filter.should_store(&make_entry("The architecture uses microservices")).await.unwrap());
        assert!(filter.should_store(&make_entry("Error: connection refused")).await.unwrap());
        assert!(filter.should_store(&make_entry("We decided to use PostgreSQL for the database")).await.unwrap());
        assert!(filter.should_store(&make_entry("User prefers TypeScript over JavaScript")).await.unwrap());
        assert!(filter.should_store(&make_entry("Important: never commit secrets to git")).await.unwrap());

        // No count incremented for pass-throughs
        assert_eq!(filter.filtered_count(), 0);
    }

    #[tokio::test]
    async fn test_category_filter_custom_patterns() {
        let custom = vec![
            r"(?i)^analyzing ".to_string(),
            r"(?i)^processing ".to_string(),
        ];
        let filter = make_category_filter(custom);

        // Custom patterns should be applied
        assert!(!filter.should_store(&make_entry("Analyzing the results now")).await.unwrap());
        assert!(!filter.should_store(&make_entry("Processing the request")).await.unwrap());

        // Built-in patterns still work
        assert!(!filter.should_store(&make_entry("Let me read the file")).await.unwrap());

        // Non-matching content passes through
        assert!(filter.should_store(&make_entry("We decided to use Redis for caching")).await.unwrap());
    }

    #[tokio::test]
    async fn test_category_filter_bad_pattern_skipped() {
        // An invalid regex pattern must not crash the filter — fail-open
        let bad_patterns = vec![
            r"[invalid regex (missing close bracket".to_string(),
            r"(?i)^valid pattern ".to_string(),
        ];
        // This must not panic
        let filter = make_category_filter(bad_patterns);

        // The valid custom pattern should still work
        assert!(!filter.should_store(&make_entry("valid pattern match")).await.unwrap());

        // Built-in patterns work too (construction succeeded despite bad pattern)
        assert!(!filter.should_store(&make_entry("Let me check the code")).await.unwrap());

        // Normal content passes through
        assert!(filter.should_store(&make_entry("We prefer Rust for performance")).await.unwrap());
    }
}
