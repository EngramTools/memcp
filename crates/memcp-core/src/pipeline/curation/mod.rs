//! AI Brain Curation — periodic self-maintenance of the memory corpus.
//!
//! Merges related entries, strengthens important ones, flags outdated ones.
//! Algorithmic-first (no LLM required), with optional LLM review for
//! contradiction detection and merge synthesis.
//! Runs as a daemon worker alongside GC.

pub mod algorithmic;
pub mod ollama;
pub mod openai;
pub mod worker;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

use crate::config::CurationConfig;

/// Errors that can occur during curation.
#[derive(Debug, Error)]
pub enum CurationError {
    /// Database or store error
    #[error("Storage error: {0}")]
    Storage(String),

    /// LLM inference or HTTP error
    #[error("LLM error: {0}")]
    Llm(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),
}

/// A memory with its salience data, used as input to curation review.
#[derive(Debug, Clone)]
pub struct ClusterMember {
    pub id: String,
    pub content: String,
    pub type_hint: Option<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub stability: f64,
    pub reinforcement_count: i32,
    pub last_reinforced_at: Option<DateTime<Utc>>,
}

/// Action the curator decides for a memory or cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CurationAction {
    /// Merge multiple memories into a single distilled entry.
    Merge {
        source_ids: Vec<String>,
        synthesized_content: String,
    },
    /// Flag a memory as stale (contradicted or low-value).
    FlagStale {
        memory_id: String,
        reason: String,
    },
    /// Strengthen an important memory (boost salience).
    Strengthen {
        memory_id: String,
        reason: String,
    },
    /// Skip — no action needed for this memory.
    Skip {
        memory_id: String,
        reason: String,
    },
}

/// Result of a complete curation run.
#[derive(Debug, Serialize)]
pub struct CurationResult {
    pub run_id: String,
    pub merged_count: usize,
    pub flagged_stale_count: usize,
    pub strengthened_count: usize,
    pub skipped_count: usize,
    pub candidates_processed: usize,
    pub clusters_found: usize,
    pub skipped_reason: Option<String>,
    /// Populated only in dry_run mode — the actions that would be taken.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proposed_actions: Vec<CurationAction>,
}

impl CurationResult {
    /// Create a skipped result with a reason.
    pub fn skipped(reason: impl Into<String>) -> Self {
        CurationResult {
            run_id: String::new(),
            merged_count: 0,
            flagged_stale_count: 0,
            strengthened_count: 0,
            skipped_count: 0,
            candidates_processed: 0,
            clusters_found: 0,
            skipped_reason: Some(reason.into()),
            proposed_actions: Vec::new(),
        }
    }
}

/// Core trait for reviewing memory clusters and deciding curation actions.
///
/// AlgorithmicCurator (default): salience-only, no LLM needed.
/// OllamaCurationProvider / OpenAICurationProvider: LLM-backed for
/// contradiction detection and merge synthesis.
#[async_trait]
pub trait CurationProvider: Send + Sync {
    /// Review a cluster of similar memories and decide actions.
    /// Returns a list of CurationActions (merge, flag-stale, strengthen, skip).
    async fn review_cluster(
        &self,
        cluster: &[ClusterMember],
    ) -> Result<Vec<CurationAction>, CurationError>;

    /// Synthesize a merged memory from multiple source memories.
    /// For algorithmic mode: concatenates content with separator.
    /// For LLM mode: produces a distilled summary.
    async fn synthesize_merge(
        &self,
        sources: &[ClusterMember],
    ) -> Result<String, CurationError>;

    /// Return the model/provider name (for provenance tracking).
    fn model_name(&self) -> &str;
}

/// Factory: create a curation LLM provider from config.
///
/// Returns None if LLM provider is not configured (algorithmic-only mode).
/// The AlgorithmicCurator is always available — this factory is for the OPTIONAL LLM layer.
pub fn create_curation_provider(
    config: &CurationConfig,
) -> Result<Option<Arc<dyn CurationProvider>>, CurationError> {
    match &config.llm_provider {
        None => Ok(None),
        Some(provider) => match provider.as_str() {
            "openai" => {
                let api_key = config.openai_api_key.clone().ok_or_else(|| {
                    CurationError::Config(
                        "OpenAI API key required when curation llm_provider is 'openai'. \
                         Set MEMCP_CURATION__OPENAI_API_KEY or curation.openai_api_key in memcp.toml"
                            .to_string(),
                    )
                })?;
                Ok(Some(Arc::new(openai::OpenAICurationProvider::new(
                    config.openai_base_url.clone(),
                    api_key,
                    config.openai_model.clone(),
                ))))
            }
            "ollama" | _ => Ok(Some(Arc::new(ollama::OllamaCurationProvider::new(
                config.ollama_base_url.clone(),
                config.ollama_model.clone(),
            )))),
        },
    }
}
