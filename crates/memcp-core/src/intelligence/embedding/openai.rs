/// OpenAI embedding provider
///
/// Calls the OpenAI Embeddings API using reqwest.
/// Model is configurable via EmbeddingConfig::openai_model.
/// Defaults to text-embedding-3-small (1536 dimensions).
/// Requires MEMCP_EMBEDDING__OPENAI_API_KEY env var or openai_api_key in config.

use async_trait::async_trait;

use super::{EmbeddingError, EmbeddingProvider, model_dimension};

/// Request body for OpenAI Embeddings API
#[derive(serde::Serialize)]
struct EmbedRequest {
    input: String,
    model: String,
}

/// Response from OpenAI Embeddings API
#[derive(serde::Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

/// Single embedding result from OpenAI
#[derive(serde::Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

/// OpenAI-compatible embedding provider.
///
/// Supports any OpenAI-compatible embeddings API (OpenAI, Google Gemini, etc.)
/// via configurable base URL. Requires a valid API key — validated on construction,
/// not at embed time.
pub struct OpenAIEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dim: usize,
    base_url: String,
}

impl OpenAIEmbeddingProvider {
    /// Create a new OpenAIEmbeddingProvider.
    ///
    /// # Arguments
    /// * `api_key` - API key (must be non-empty)
    /// * `model` - Model name; defaults to "text-embedding-3-small" if None
    /// * `dimension` - Override vector dimension; auto-detected from model if None
    /// * `base_url` - API base URL; defaults to "https://api.openai.com/v1" if None.
    ///   For Google Gemini: "https://generativelanguage.googleapis.com/v1beta/openai"
    ///
    /// # Errors
    /// Returns `EmbeddingError::NotConfigured` if api_key is empty.
    /// Returns `EmbeddingError::ModelInit` if model is unknown and no dimension override provided.
    pub fn new(
        api_key: String,
        model: Option<String>,
        dimension: Option<usize>,
        base_url: Option<String>,
    ) -> Result<Self, EmbeddingError> {
        if api_key.trim().is_empty() {
            return Err(EmbeddingError::NotConfigured(
                "API key is required when using the openai embedding provider. \
                 Set MEMCP_EMBEDDING__OPENAI_API_KEY or openai_api_key in memcp.toml"
                    .to_string(),
            ));
        }

        let model_name = model.unwrap_or_else(|| "text-embedding-3-small".to_string());
        let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        // Resolve dimension: explicit override > registry lookup > error for unknown models
        let dim = match dimension {
            Some(d) => d,
            None => model_dimension(&model_name).ok_or_else(|| {
                EmbeddingError::ModelInit(format!(
                    "Unknown model '{}'. Provide 'embedding.dimension' in config to override, \
                     or use a known model: text-embedding-3-small, text-embedding-3-large, \
                     text-embedding-ada-002, gemini-embedding-001",
                    model_name
                ))
            })?,
        };

        Ok(OpenAIEmbeddingProvider {
            client: reqwest::Client::new(),
            api_key,
            model: model_name,
            dim,
            base_url,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let request = EmbedRequest {
            input: text.to_string(),
            model: self.model.clone(),
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| EmbeddingError::Generation(format!("HTTP request failed: {}", e)))?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(EmbeddingError::Api {
                status,
                message: body,
            });
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::Generation(format!("Failed to parse API response: {}", e)))?;

        embed_response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| EmbeddingError::Generation("API returned empty embedding list".to_string()))
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}
