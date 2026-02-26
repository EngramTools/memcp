/// Filter strategy trait and implementations for auto-store sidecar.
///
/// Decides whether a parsed log entry is worth storing as a memory.
/// Four modes: LLM-based, heuristic keyword matching, category-aware, or no filtering.

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;

use crate::errors::MemcpError;
use super::parser::ParsedEntry;

/// Category classification result from LLM or heuristic.
#[derive(Debug, Clone)]
pub struct CategoryResult {
    pub category: String,
    pub action: String,  // "store", "skip", "store-low"
}

/// Trait for deciding whether a parsed entry should be stored.
#[async_trait]
pub trait FilterStrategy: Send + Sync {
    /// Returns true if the entry contains information worth remembering.
    async fn should_store(&self, entry: &ParsedEntry) -> Result<bool, MemcpError>;

    /// Get the last classification result (if any). Only CategoryFilter with LLM returns Some.
    fn last_classification(&self) -> Option<CategoryResult> { None }
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

/// LLM-based category classifier.
///
/// Sends content to an LLM with a taxonomy prompt and parses the response
/// into one of 10 predefined categories. Each category maps to a configurable
/// action (store, skip, store-low). Falls back to "store" for unknown categories.
pub struct LlmCategoryClassifier {
    client: reqwest::Client,
    base_url: String,
    model: String,
    provider: String,
    api_key: Option<String>,
    category_actions: std::collections::HashMap<String, String>,
}

impl LlmCategoryClassifier {
    pub fn new(
        provider: String,
        base_url: String,
        model: String,
        api_key: Option<String>,
        category_actions: std::collections::HashMap<String, String>,
    ) -> Self {
        LlmCategoryClassifier {
            client: reqwest::Client::new(),
            base_url,
            model,
            provider,
            api_key,
            category_actions,
        }
    }

    fn build_classification_prompt(content: &str) -> String {
        let truncated = &content[..content.len().min(2000)];
        format!(
            "Classify the following text into exactly ONE category. \
            Respond with ONLY the category name, nothing else.\n\n\
            Categories:\n\
            - decision: A choice or decision made\n\
            - preference: A user preference or setting\n\
            - architecture: Technical architecture or design pattern\n\
            - fact: A factual statement worth remembering\n\
            - instruction: An instruction or rule to follow\n\
            - correction: A correction to previous information\n\
            - tool-narration: Narration of tool usage (e.g. 'Let me read the file...')\n\
            - ephemeral: Temporary or transient content\n\
            - code-output: Raw code output or terminal results\n\
            - error-trace: Error messages or stack traces\n\n\
            Text:\n{}",
            truncated
        )
    }

    /// Classify content into a category. Returns None on failure (fail-open).
    pub async fn classify(&self, content: &str) -> Option<CategoryResult> {
        let prompt = Self::build_classification_prompt(content);

        let response_text = match self.provider.as_str() {
            "openai" => {
                let api_key = self.api_key.as_deref()?;
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": [{"role": "user", "content": prompt}],
                    "max_tokens": 10,
                    "temperature": 0.0
                });
                let resp = self
                    .client
                    .post(format!("{}/chat/completions", self.base_url))
                    .header("Authorization", format!("Bearer {}", api_key))
                    .json(&body)
                    .send()
                    .await
                    .ok()?;

                if !resp.status().is_success() {
                    tracing::warn!(status = %resp.status(), "LLM category classifier API error");
                    return None;
                }

                let parsed: OpenAIResponse = resp.json().await.ok()?;
                parsed.choices.first()?.message.content.clone()?
            }
            _ => {
                // Ollama
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": [{"role": "user", "content": prompt}],
                    "stream": false,
                    "options": {"temperature": 0.0, "num_predict": 10}
                });
                let resp = self
                    .client
                    .post(format!("{}/api/chat", self.base_url))
                    .json(&body)
                    .send()
                    .await
                    .ok()?;

                if !resp.status().is_success() {
                    tracing::warn!(status = %resp.status(), "LLM category classifier API error");
                    return None;
                }

                let parsed: OllamaChatResponse = resp.json().await.ok()?;
                parsed.message.content
            }
        };

        let category = response_text.trim().to_lowercase().replace(' ', "-");

        // Look up action for this category, default to "store" for unknown categories
        let action = self.category_actions
            .get(&category)
            .cloned()
            .unwrap_or_else(|| "store".to_string());

        Some(CategoryResult { category, action })
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
/// When an `LlmCategoryClassifier` is provided, it classifies content into a
/// 10-category taxonomy (decision, preference, architecture, fact, instruction,
/// correction, tool-narration, ephemeral, code-output, error-trace). Each category
/// maps to a configurable action (store, skip, store-low). Falls back to heuristic
/// patterns when LLM is unavailable or times out.
pub struct CategoryFilter {
    patterns: Vec<Regex>,
    llm_classifier: Option<LlmCategoryClassifier>,
    filtered_count: Arc<AtomicU64>,
    last_result: Arc<Mutex<Option<CategoryResult>>>,
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
    /// An optional `LlmCategoryClassifier` enables rich category taxonomy classification.
    /// When provided, content is classified into one of 10 categories with configurable
    /// per-category actions. When absent, only heuristic patterns are used.
    pub fn new(
        config: &crate::config::CategoryFilterConfig,
        llm_classifier: Option<LlmCategoryClassifier>,
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

        if llm_classifier.is_some() {
            tracing::info!("CategoryFilter: LLM category classifier enabled");
        }

        CategoryFilter {
            patterns,
            llm_classifier,
            filtered_count: Arc::new(AtomicU64::new(0)),
            last_result: Arc::new(Mutex::new(None)),
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
        // Clear last classification
        *self.last_result.lock().await = None;

        // Step 1: Try LLM classification if available (with 3s timeout)
        if let Some(ref classifier) = self.llm_classifier {
            match tokio::time::timeout(
                std::time::Duration::from_secs(3),
                classifier.classify(&entry.content),
            ).await {
                Ok(Some(result)) => {
                    tracing::debug!(
                        category = %result.category,
                        action = %result.action,
                        content_preview = %entry.content.chars().take(80).collect::<String>(),
                        "LLM category classification"
                    );
                    let should_store = result.action != "skip";
                    if !should_store {
                        self.filtered_count.fetch_add(1, Ordering::Relaxed);
                    }
                    // Cache classification for the auto-store worker to read
                    *self.last_result.lock().await = Some(result);
                    return Ok(should_store);
                }
                Ok(None) => {
                    tracing::debug!("LLM category classifier returned None, falling back to heuristic");
                }
                Err(_) => {
                    tracing::warn!("LLM category classifier timed out (3s), falling back to heuristic");
                }
            }
        }

        // Step 2: Heuristic fallback — check tool narration patterns
        for pattern in &self.patterns {
            if pattern.is_match(&entry.content) {
                self.filtered_count.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    content = %entry.content.chars().take(80).collect::<String>(),
                    "Filtered: tool narration (heuristic)"
                );
                return Ok(false);
            }
        }

        // Pass through anything that doesn't match patterns
        Ok(true)
    }

    fn last_classification(&self) -> Option<CategoryResult> {
        // Use try_lock to avoid blocking — if locked, return None
        self.last_result.try_lock().ok().and_then(|guard| guard.clone())
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
            let llm_classifier = auto_store_config.category_filter.llm_provider.as_ref().map(|provider| {
                let model = auto_store_config.category_filter.llm_model.clone()
                    .unwrap_or_else(|| model.to_string());
                let (base_url, api_key) = match provider.as_str() {
                    "openai" => (
                        "https://api.openai.com/v1".to_string(),
                        extraction_config.openai_api_key.clone(),
                    ),
                    _ => (
                        extraction_config.ollama_base_url.clone(),
                        None,
                    ),
                };
                LlmCategoryClassifier::new(
                    provider.clone(),
                    base_url,
                    model,
                    api_key,
                    auto_store_config.category_filter.category_actions.clone(),
                )
            });
            Box::new(CategoryFilter::new(&auto_store_config.category_filter, llm_classifier))
        }
        "heuristic" => Box::new(HeuristicFilter),
        "none" => Box::new(NoFilter),
        other => {
            tracing::warn!(mode = other, "Unknown filter mode, falling back to heuristic");
            Box::new(HeuristicFilter)
        }
    }
}

