//! Multi-tier embedding router.
//!
//! Routes embedding jobs to the correct provider based on memory metadata
//! (type_hint, stability, content length). Supports single-tier (backward compat)
//! and multi-tier (e.g., fast local + quality API) configurations.

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;

use super::{EmbeddingError, EmbeddingProvider};
use crate::config::RoutingConfig;

/// Entry for a single embedding tier.
struct TierEntry {
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    routing: Option<RoutingConfig>,
}

/// Routes embedding jobs to the correct provider based on memory metadata.
///
/// In single-tier mode (one tier), all memories are embedded by the default provider.
/// In multi-tier mode, routing rules determine which tier handles each memory.
pub struct EmbeddingRouter {
    /// Tier name -> provider + routing rules
    tiers: HashMap<String, TierEntry>,
    /// Default tier name (fallback when no routing rules match)
    default_tier: String,
}

impl EmbeddingRouter {
    /// Create a new router from a map of tier names to (provider, routing rules) pairs.
    pub fn new(
        tiers: HashMap<String, (Arc<dyn EmbeddingProvider + Send + Sync>, Option<RoutingConfig>)>,
        default_tier: String,
    ) -> Self {
        let entries = tiers
            .into_iter()
            .map(|(name, (provider, routing))| {
                (name, TierEntry { provider, routing })
            })
            .collect();
        EmbeddingRouter {
            tiers: entries,
            default_tier,
        }
    }

    /// Select the appropriate tier for a memory based on its metadata.
    ///
    /// Logic:
    /// 1. If only one tier, return default.
    /// 2. Check each non-default tier's routing rules. A tier matches if ALL
    ///    specified conditions are met (min_stability AND type_hints AND min_content_length).
    /// 3. If no non-default tier matches, return default.
    pub fn route(&self, type_hint: Option<&str>, stability: Option<f64>, content_length: usize) -> &str {
        if self.tiers.len() <= 1 {
            return &self.default_tier;
        }

        for (name, entry) in &self.tiers {
            if name == &self.default_tier {
                continue;
            }
            if let Some(ref routing) = entry.routing {
                if self.matches_routing(routing, type_hint, stability, content_length) {
                    return name;
                }
            }
        }

        &self.default_tier
    }

    /// Check if all specified routing conditions are met.
    fn matches_routing(
        &self,
        routing: &RoutingConfig,
        type_hint: Option<&str>,
        stability: Option<f64>,
        content_length: usize,
    ) -> bool {
        // Check min_stability (if specified, stability must be present and >= threshold)
        if let Some(min_stab) = routing.min_stability {
            match stability {
                Some(s) if s >= min_stab => {}
                _ => return false,
            }
        }

        // Check type_hints (if non-empty, type_hint must be present and in the list)
        if !routing.type_hints.is_empty() {
            match type_hint {
                Some(th) if routing.type_hints.iter().any(|t| t == th) => {}
                _ => return false,
            }
        }

        // Check min_content_length (if specified, content must be long enough)
        if let Some(min_len) = routing.min_content_length {
            if content_length < min_len {
                return false;
            }
        }

        true
    }

    /// Get the provider for a specific tier by name.
    pub fn provider(&self, tier: &str) -> Option<&Arc<dyn EmbeddingProvider + Send + Sync>> {
        self.tiers.get(tier).map(|e| &e.provider)
    }

    /// Get the default tier's provider.
    pub fn default_provider(&self) -> &Arc<dyn EmbeddingProvider + Send + Sync> {
        &self.tiers[&self.default_tier].provider
    }

    /// List all tier names.
    pub fn tier_names(&self) -> Vec<&str> {
        self.tiers.keys().map(|s| s.as_str()).collect()
    }

    /// Get tier name -> dimension mapping (for HNSW index creation).
    pub fn tier_dimensions(&self) -> HashMap<&str, usize> {
        self.tiers
            .iter()
            .map(|(name, entry)| (name.as_str(), entry.provider.dimension()))
            .collect()
    }

    /// Returns true if multiple tiers are configured.
    pub fn is_multi_model(&self) -> bool {
        self.tiers.len() > 1
    }

    /// Get the default tier name.
    pub fn default_tier_name(&self) -> &str {
        &self.default_tier
    }

    /// Embed a query with all active tiers. Returns tier_name -> Vector mapping.
    ///
    /// Skips non-default tiers that have zero embeddings in the corpus (lazy optimization)
    /// to avoid unnecessary API calls. If a non-default tier's embedding fails, it is
    /// skipped silently. If the default tier fails, the error is propagated.
    pub async fn embed_query_all_tiers(
        &self,
        text: &str,
        store: &crate::store::postgres::PostgresMemoryStore,
    ) -> Result<std::collections::HashMap<String, pgvector::Vector>, EmbeddingError> {
        let mut results = std::collections::HashMap::new();

        for (tier_name, entry) in &self.tiers {
            // Lazy check: skip non-default tier embedding if no memories use it yet
            if tier_name != &self.default_tier {
                match store.count_tier_embeddings(tier_name).await {
                    Ok(0) => {
                        tracing::debug!(tier = %tier_name, "Skipping query embedding — no memories in tier");
                        continue;
                    }
                    Ok(_) => {} // has memories, proceed
                    Err(e) => {
                        tracing::warn!(tier = %tier_name, error = %e, "Failed to check tier count, skipping");
                        continue;
                    }
                }
            }

            match entry.provider.embed(text).await {
                Ok(vector) => {
                    results.insert(tier_name.clone(), pgvector::Vector::from(vector));
                }
                Err(e) => {
                    // Non-default tiers fail silently; default tier failure is an error
                    if tier_name == &self.default_tier {
                        return Err(e);
                    }
                    tracing::warn!(tier = %tier_name, error = %e, "Failed to embed query for tier");
                }
            }
        }

        Ok(results)
    }
}

/// EmbeddingRouter implements EmbeddingProvider by delegating to the default tier.
///
/// This allows it to be used as a drop-in replacement in contexts that expect
/// a single `Arc<dyn EmbeddingProvider>` (e.g., content filter, IPC listener).
#[async_trait]
impl EmbeddingProvider for EmbeddingRouter {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.default_provider().embed(text).await
    }

    fn model_name(&self) -> &str {
        self.default_provider().model_name()
    }

    fn dimension(&self) -> usize {
        self.default_provider().dimension()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock provider for testing routing logic
    struct MockProvider {
        name: String,
        dim: usize,
    }

    #[async_trait]
    impl EmbeddingProvider for MockProvider {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Ok(vec![0.0; self.dim])
        }
        fn model_name(&self) -> &str {
            &self.name
        }
        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn make_router() -> EmbeddingRouter {
        let fast: Arc<dyn EmbeddingProvider + Send + Sync> = Arc::new(MockProvider {
            name: "fast-model".to_string(),
            dim: 384,
        });
        let quality: Arc<dyn EmbeddingProvider + Send + Sync> = Arc::new(MockProvider {
            name: "quality-model".to_string(),
            dim: 1536,
        });

        let mut tiers = HashMap::new();
        tiers.insert("fast".to_string(), (fast, None));
        tiers.insert(
            "quality".to_string(),
            (
                quality,
                Some(RoutingConfig {
                    min_stability: Some(0.8),
                    type_hints: vec!["decision".to_string()],
                    min_content_length: None,
                }),
            ),
        );

        EmbeddingRouter::new(tiers, "fast".to_string())
    }

    #[test]
    fn single_tier_always_returns_default() {
        let provider: Arc<dyn EmbeddingProvider + Send + Sync> = Arc::new(MockProvider {
            name: "only".to_string(),
            dim: 384,
        });
        let mut tiers = HashMap::new();
        tiers.insert("fast".to_string(), (provider, None));
        let router = EmbeddingRouter::new(tiers, "fast".to_string());

        assert_eq!(router.route(Some("decision"), Some(1.0), 5000), "fast");
        assert!(!router.is_multi_model());
    }

    #[test]
    fn routes_to_quality_when_all_conditions_met() {
        let router = make_router();
        // Both conditions met: type_hint="decision" AND stability=0.9 >= 0.8
        assert_eq!(router.route(Some("decision"), Some(0.9), 100), "quality");
    }

    #[test]
    fn routes_to_fast_when_type_hint_missing() {
        let router = make_router();
        // stability met but type_hint missing
        assert_eq!(router.route(None, Some(0.9), 100), "fast");
    }

    #[test]
    fn routes_to_fast_when_stability_too_low() {
        let router = make_router();
        // type_hint matches but stability too low
        assert_eq!(router.route(Some("decision"), Some(0.5), 100), "fast");
    }

    #[test]
    fn routes_to_fast_when_no_stability() {
        let router = make_router();
        // type_hint matches but no stability available (new memory)
        assert_eq!(router.route(Some("decision"), None, 100), "fast");
    }

    #[test]
    fn tier_dimensions_returns_all_tiers() {
        let router = make_router();
        let dims = router.tier_dimensions();
        assert_eq!(dims["fast"], 384);
        assert_eq!(dims["quality"], 1536);
    }

    #[test]
    fn is_multi_model_correct() {
        let router = make_router();
        assert!(router.is_multi_model());
    }

    #[test]
    fn default_provider_returns_fast() {
        let router = make_router();
        assert_eq!(router.default_provider().model_name(), "fast-model");
        // EmbeddingProvider impl delegates to default
        assert_eq!(router.model_name(), "fast-model");
        assert_eq!(router.dimension(), 384);
    }
}
