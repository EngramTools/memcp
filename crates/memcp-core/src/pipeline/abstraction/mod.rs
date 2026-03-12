//! Tiered content abstraction pipeline.
//!
//! Generates L0 abstracts (~100 tokens) and optional L1 overviews (~500 tokens)
//! for memory entries, improving semantic search quality and enabling tiered
//! context loading for agent consumers.
//!
//! AbstractionProvider trait with Ollama and OpenAI implementations.
//! Used by the abstraction daemon worker to process memories with abstraction_status='pending'.

pub mod ollama;
pub mod openai;
pub mod worker;

use async_trait::async_trait;
use std::sync::Arc;
use thiserror::Error;

use crate::config::AbstractionConfig;

/// Errors that can occur during abstraction generation.
#[derive(Debug, Error)]
pub enum AbstractionError {
    /// LLM inference or HTTP error
    #[error("Abstraction generation failed: {0}")]
    Generation(String),

    /// API provider returned an HTTP error
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    /// Provider not configured (e.g., missing API key)
    #[error("Provider not configured: {0}")]
    NotConfigured(String),
}

/// Core trait for generating tiered content abstractions.
///
/// Implementations must be Send + Sync for use in async/multi-thread contexts.
/// Both L0 (abstract) and L1 (overview) generation methods are provided,
/// using their respective prompt templates from AbstractionConfig.
#[async_trait]
pub trait AbstractionProvider: Send + Sync {
    /// Generate a single concise sentence (~100 tokens) capturing key information.
    ///
    /// This is the L0 abstract — optimized for semantic search quality.
    async fn generate_abstract(&self, content: &str) -> Result<String, AbstractionError>;

    /// Generate a structured overview in 3-5 bullet points (~500 tokens).
    ///
    /// This is the L1 overview — provides mid-level context without full content.
    async fn generate_overview(&self, content: &str) -> Result<String, AbstractionError>;

    /// Return the model name used by this provider.
    fn model_name(&self) -> &str;
}

/// Create an abstraction provider from configuration.
///
/// Returns None if abstraction is disabled.
/// Returns Err if enabled but misconfigured (e.g., openai without API key).
pub fn create_abstraction_provider(
    config: &AbstractionConfig,
) -> Result<Option<Arc<dyn AbstractionProvider>>, AbstractionError> {
    if !config.enabled {
        return Ok(None);
    }

    let provider: Arc<dyn AbstractionProvider> = match config.provider.as_str() {
        "openai" => {
            let api_key = config.openai_api_key.clone().ok_or_else(|| {
                AbstractionError::NotConfigured(
                    "OpenAI API key required when abstraction provider is 'openai'. \
                     Set MEMCP_ABSTRACTION__OPENAI_API_KEY or abstraction.openai_api_key in memcp.toml"
                        .to_string(),
                )
            })?;
            Arc::new(openai::OpenAIAbstractionProvider::new(
                config.openai_base_url.clone(),
                api_key,
                config.openai_model.clone(),
                config.max_input_chars,
                config.abstract_prompt_template.clone(),
                config.overview_prompt_template.clone(),
            ))
        }
        _ => Arc::new(ollama::OllamaAbstractionProvider::new(
            config.ollama_base_url.clone(),
            config.ollama_model.clone(),
            config.max_input_chars,
            config.abstract_prompt_template.clone(),
            config.overview_prompt_template.clone(),
        )),
    };

    Ok(Some(provider))
}
