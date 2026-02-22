/// Local embedding provider using fastembed
///
/// Provides offline embedding generation using configurable fastembed models.
/// No API key required — model weights are downloaded and cached locally.
/// All CPU-bound fastembed calls are wrapped in spawn_blocking to avoid blocking async runtime.

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::task;

use super::{EmbeddingError, EmbeddingProvider, model_dimension};

/// Local embedding provider backed by fastembed.
///
/// Model is configurable via `EmbeddingConfig::local_model`.
/// Defaults to AllMiniLML6V2 (384 dimensions, all-MiniLM-L6-v2).
/// fastembed is synchronous, so embed() uses spawn_blocking internally.
pub struct LocalEmbeddingProvider {
    model: Arc<Mutex<TextEmbedding>>,
    name: String,
    dim: usize,
}

/// Map a model name string to the fastembed EmbeddingModel enum variant.
fn resolve_fastembed_model(name: &str) -> Result<EmbeddingModel, EmbeddingError> {
    match name {
        "AllMiniLML6V2" | "all-MiniLM-L6-v2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "BGESmallENV15" | "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "AllMiniLML12V2" | "all-MiniLM-L12-v2" => Ok(EmbeddingModel::AllMiniLML12V2),
        "BGEBaseENV15" | "bge-base-en-v1.5" => Ok(EmbeddingModel::BGEBaseENV15),
        "BGELargeENV15" | "bge-large-en-v1.5" => Ok(EmbeddingModel::BGELargeENV15),
        _ => Err(EmbeddingError::ModelInit(format!(
            "Unknown local model: '{}'. Supported: AllMiniLML6V2, BGESmallENV15, AllMiniLML12V2, BGEBaseENV15, BGELargeENV15",
            name
        ))),
    }
}

impl LocalEmbeddingProvider {
    /// Create a new LocalEmbeddingProvider, downloading model weights if not cached.
    ///
    /// # Arguments
    /// * `cache_dir` - Directory to cache model weights (fastembed downloads on first use)
    /// * `model_name` - fastembed model identifier (e.g. "AllMiniLML6V2", "bge-base-en-v1.5")
    pub async fn new(cache_dir: &str, model_name: &str) -> Result<Self, EmbeddingError> {
        let fastembed_model = resolve_fastembed_model(model_name)?;
        let dim = model_dimension(model_name).ok_or_else(|| {
            EmbeddingError::ModelInit(format!(
                "No dimension known for model '{}' — this is a bug in the dimension registry",
                model_name
            ))
        })?;

        // Canonical display name: prefer the canonical alias if caller used a short name
        let display_name = model_name.to_string();

        let cache_path = PathBuf::from(cache_dir);

        let te = task::spawn_blocking(move || {
            TextEmbedding::try_new(
                InitOptions::new(fastembed_model)
                    .with_cache_dir(cache_path)
                    .with_show_download_progress(true),
            )
        })
        .await
        .map_err(|e| EmbeddingError::ModelInit(e.to_string()))?
        .map_err(|e| EmbeddingError::ModelInit(e.to_string()))?;

        Ok(LocalEmbeddingProvider {
            model: Arc::new(Mutex::new(te)),
            name: display_name,
            dim,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let model = Arc::clone(&self.model);
        let text = text.to_string();

        task::spawn_blocking(move || {
            let mut model = model.lock().unwrap();
            let mut embeddings = model
                .embed(vec![text], None)
                .map_err(|e| EmbeddingError::Generation(e.to_string()))?;

            embeddings
                .pop()
                .ok_or_else(|| EmbeddingError::Generation("fastembed returned empty result".to_string()))
        })
        .await
        .map_err(|e| EmbeddingError::Generation(format!("spawn_blocking panicked: {}", e)))?
    }

    fn model_name(&self) -> &str {
        &self.name
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}
