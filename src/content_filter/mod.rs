/// Content filtering module for ingestion-time topic exclusion.
///
/// Two-tier system: regex patterns (fast, deterministic) then semantic
/// topic exclusion (embedding-based cosine similarity). CompositeFilter
/// short-circuits on first Drop verdict.

pub mod regex_filter;
pub mod semantic_filter;

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::ContentFilterConfig;
use crate::embedding::EmbeddingProvider;
use crate::errors::MemcpError;

use self::regex_filter::RegexFilter;
use self::semantic_filter::SemanticTopicFilter;

/// Outcome of a content filter check.
#[derive(Debug, Clone)]
pub enum FilterVerdict {
    /// Content is allowed through
    Allow,
    /// Content was silently dropped
    Drop { reason: String },
}

/// Trait for content filter implementations.
#[async_trait]
pub trait ContentFilter: Send + Sync {
    /// Check whether content should be stored.
    async fn check(&self, content: &str) -> Result<FilterVerdict, MemcpError>;
}

/// Composite filter: runs regex first (fast), then semantic (slower).
/// Short-circuits on first Drop verdict.
pub struct CompositeFilter {
    regex_filter: Option<RegexFilter>,
    semantic_filter: Option<SemanticTopicFilter>,
}

impl CompositeFilter {
    /// Build a CompositeFilter from config and an optional embedding provider.
    ///
    /// - Regex patterns are compiled immediately (fails fast on invalid patterns).
    /// - Semantic topics are embedded using the provider (requires provider to be Some).
    /// - If no patterns and no topics configured, both inner filters are None.
    pub async fn from_config(
        config: &ContentFilterConfig,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Self, MemcpError> {
        let regex_filter = if config.regex_patterns.is_empty() {
            None
        } else {
            Some(RegexFilter::new(&config.regex_patterns)?)
        };

        let semantic_filter = if config.excluded_topics.is_empty() {
            None
        } else if let Some(provider) = embedding_provider {
            Some(
                SemanticTopicFilter::new(
                    &config.excluded_topics,
                    config.semantic_threshold,
                    provider,
                )
                .await?,
            )
        } else {
            tracing::warn!(
                "Semantic topic exclusion configured but no embedding provider available — skipping"
            );
            None
        };

        Ok(CompositeFilter {
            regex_filter,
            semantic_filter,
        })
    }
}

#[async_trait]
impl ContentFilter for CompositeFilter {
    async fn check(&self, content: &str) -> Result<FilterVerdict, MemcpError> {
        // Tier 1: Regex (fast, synchronous)
        if let Some(ref regex) = self.regex_filter {
            if let Some(pattern) = regex.matches(content) {
                return Ok(FilterVerdict::Drop {
                    reason: format!("Content matched exclusion pattern: {}", pattern),
                });
            }
        }

        // Tier 2: Semantic (slower, requires embedding)
        if let Some(ref semantic) = self.semantic_filter {
            if let Some(topic) = semantic.check_content(content).await? {
                return Ok(FilterVerdict::Drop {
                    reason: format!("Content matched excluded topic: {}", topic),
                });
            }
        }

        Ok(FilterVerdict::Allow)
    }
}
