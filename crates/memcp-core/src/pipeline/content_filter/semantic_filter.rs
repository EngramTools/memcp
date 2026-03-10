//! Semantic topic exclusion filter using embedding cosine similarity.
//!
//! Embeds excluded topics at construction time and compares incoming content
//! embeddings via cosine similarity. Uses the existing EmbeddingProvider.

use std::sync::Arc;

use crate::embedding::EmbeddingProvider;
use crate::errors::MemcpError;

/// Semantic topic exclusion filter.
///
/// Pre-embeds excluded topics at startup. For each incoming content,
/// embeds it and compares against all topic embeddings using cosine similarity.
/// If similarity exceeds threshold, content is excluded.
pub struct SemanticTopicFilter {
    /// Pre-computed topic embeddings: (topic_name, embedding_vector)
    topic_embeddings: Vec<(String, Vec<f32>)>,
    /// Cosine similarity threshold — content above this is excluded
    threshold: f64,
    /// Embedding provider for embedding incoming content
    provider: Arc<dyn EmbeddingProvider>,
}

impl SemanticTopicFilter {
    /// Create a new SemanticTopicFilter by embedding all excluded topics.
    ///
    /// Takes an Arc'd provider (stored for runtime content embedding).
    /// Embeds each topic string at construction. Fails if any topic fails to embed.
    pub async fn new(
        topics: &[String],
        threshold: f64,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Result<Self, MemcpError> {
        let mut topic_embeddings = Vec::with_capacity(topics.len());
        for topic in topics {
            let embedding = provider.embed(topic).await.map_err(|e| {
                MemcpError::Config(format!(
                    "Failed to embed exclusion topic '{}': {}",
                    topic, e
                ))
            })?;
            topic_embeddings.push((topic.clone(), embedding));
        }
        tracing::info!(
            topic_count = topics.len(),
            threshold = threshold,
            "Content filter: semantic topics embedded"
        );
        Ok(SemanticTopicFilter {
            topic_embeddings,
            threshold,
            provider,
        })
    }

    /// Check if content matches any excluded topic.
    ///
    /// Embeds the content, then compares against all topic embeddings.
    /// Returns the matched topic name if similarity exceeds threshold, None otherwise.
    pub async fn check_content(&self, content: &str) -> Result<Option<String>, MemcpError> {
        let embed_result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            self.provider.embed(content),
        )
        .await;

        let content_embedding = match embed_result {
            Ok(Ok(emb)) => emb,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Semantic filter: failed to embed content, allowing through");
                return Ok(None);
            }
            Err(_) => {
                tracing::warn!("Semantic filter: embedding timed out (100ms), allowing through");
                return Ok(None);
            }
        };

        for (topic, topic_emb) in &self.topic_embeddings {
            let sim = cosine_similarity(&content_embedding, topic_emb);
            if sim >= self.threshold {
                return Ok(Some(topic.clone()));
            }
        }

        Ok(None)
    }
}

/// Compute cosine similarity between two vectors.
///
/// Returns 0.0 if either vector has zero norm (degenerate case).
/// Compute cosine similarity between two f32 vectors. Exposed as `pub` for external test access.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    (dot / (norm_a * norm_b)) as f64
}
