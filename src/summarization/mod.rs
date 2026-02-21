/// Summarization provider trait and supporting types.
///
/// Provides a pluggable interface for summarizing AI assistant responses
/// before storage. Supports Ollama (local, default) and OpenAI.

pub mod ollama;
pub mod openai;

use async_trait::async_trait;
use thiserror::Error;
use std::sync::Arc;

use crate::config::SummarizationConfig;

/// Errors that can occur during summarization.
#[derive(Debug, Error)]
pub enum SummarizationError {
    /// LLM inference or HTTP error
    #[error("Summarization failed: {0}")]
    Generation(String),

    /// API provider returned an HTTP error
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    /// Provider not configured (e.g., missing API key)
    #[error("Provider not configured: {0}")]
    NotConfigured(String),
}

/// Core trait for summarizing text content.
///
/// Implementations must be Send + Sync for use in async/multi-thread contexts.
#[async_trait]
pub trait SummarizationProvider: Send + Sync {
    /// Summarize the given content into a concise memory entry.
    async fn summarize(&self, content: &str) -> Result<String, SummarizationError>;

    /// Return the model name used by this provider.
    fn model_name(&self) -> &str;
}

/// Create a summarization provider from configuration.
///
/// Returns None if summarization is disabled.
/// Returns Err if enabled but misconfigured (e.g., openai without API key).
pub fn create_summarization_provider(
    config: &SummarizationConfig,
) -> Result<Option<Arc<dyn SummarizationProvider>>, SummarizationError> {
    if !config.enabled {
        return Ok(None);
    }

    let provider: Arc<dyn SummarizationProvider> = match config.provider.as_str() {
        "openai" => {
            let api_key = config.openai_api_key.clone().ok_or_else(|| {
                SummarizationError::NotConfigured(
                    "OpenAI API key required when summarization provider is 'openai'. \
                     Set MEMCP_SUMMARIZATION__OPENAI_API_KEY or summarization.openai_api_key in memcp.toml"
                        .to_string(),
                )
            })?;
            Arc::new(openai::OpenAISummarizationProvider::new(
                config.openai_base_url.clone(),
                api_key,
                config.openai_model.clone(),
                config.max_input_chars,
                config.prompt_template.clone(),
            ))
        }
        "ollama" | _ => Arc::new(ollama::OllamaSummarizationProvider::new(
            config.ollama_base_url.clone(),
            config.ollama_model.clone(),
            config.max_input_chars,
            config.prompt_template.clone(),
        )),
    };

    Ok(Some(provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_provider_disabled() {
        let config = SummarizationConfig::default(); // enabled: false
        let result = create_summarization_provider(&config).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_create_provider_ollama() {
        let mut config = SummarizationConfig::default();
        config.enabled = true;
        let result = create_summarization_provider(&config).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().model_name(), "llama3.2:3b");
    }

    #[test]
    fn test_create_provider_openai_missing_key() {
        let mut config = SummarizationConfig::default();
        config.enabled = true;
        config.provider = "openai".to_string();
        let result = create_summarization_provider(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_provider_openai_with_key() {
        let mut config = SummarizationConfig::default();
        config.enabled = true;
        config.provider = "openai".to_string();
        config.openai_api_key = Some("sk-test".to_string());
        let result = create_summarization_provider(&config).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().model_name(), "gpt-4o-mini");
    }
}
