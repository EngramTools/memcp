//! Promotion sweep worker — re-embeds important memories with the quality model.
//!
//! Called periodically by the daemon. Fetches candidate memories from the DB
//! that currently have fast-tier embeddings and meet promotion thresholds,
//! then re-embeds them with the quality provider.

use std::sync::Arc;
use uuid::Uuid;

use metrics;

use super::PromotionResult;
use crate::config::PromotionConfig;
use crate::embedding::{build_embedding_text, EmbeddingProvider};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::MemoryStore;

/// Run a single promotion sweep cycle.
///
/// Fetches candidates from `source_tier` that meet the promotion thresholds,
/// embeds them with `quality_provider`, and moves them to `target_tier`.
///
/// Fail-open: individual promotion failures are logged and skipped.
/// The sweep continues processing remaining candidates.
pub async fn run_promotion_sweep(
    store: &PostgresMemoryStore,
    quality_provider: &Arc<dyn EmbeddingProvider + Send + Sync>,
    promotion_config: &PromotionConfig,
    source_tier: &str,
    target_tier: &str,
) -> Result<PromotionResult, anyhow::Error> {
    // Fetch candidates that meet promotion thresholds
    let candidates = store
        .get_promotion_candidates(
            promotion_config.min_stability,
            promotion_config.min_reinforcements as i32,
            source_tier,
            promotion_config.batch_cap as i64,
        )
        .await?;

    if candidates.is_empty() {
        return Ok(PromotionResult {
            promoted_count: 0,
            candidates_evaluated: 0,
            failed_count: 0,
            skipped_reason: Some("No promotion candidates found".to_string()),
        });
    }

    let candidates_evaluated = candidates.len();
    let mut promoted_count = 0;
    let mut failed_count = 0;

    for memory_id in &candidates {
        // Fetch memory content for embedding
        let memory = match store.get(memory_id).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    memory_id = %memory_id,
                    error = %e,
                    "Failed to fetch memory for promotion — skipping"
                );
                failed_count += 1;
                continue;
            }
        };

        // Build embedding text (content + tags)
        let text = build_embedding_text(&memory.content, &memory.tags);

        // Embed with quality provider
        match quality_provider.embed(&text).await {
            Ok(vector) => {
                let embedding = pgvector::Vector::from(vector);
                let emb_id = Uuid::new_v4().to_string();
                let model = quality_provider.model_name().to_string();
                let dim = quality_provider.dimension() as i32;

                // Deactivate the old source-tier embedding
                if let Err(e) = store
                    .deactivate_tier_embedding(memory_id, source_tier)
                    .await
                {
                    tracing::warn!(
                        memory_id = %memory_id,
                        error = %e,
                        "Failed to deactivate old embedding — skipping promotion"
                    );
                    failed_count += 1;
                    continue;
                }

                // Insert the new quality-tier embedding
                if let Err(e) = store
                    .insert_embedding(
                        &emb_id,
                        memory_id,
                        &model,
                        "v1",
                        dim,
                        &embedding,
                        true,
                        target_tier,
                    )
                    .await
                {
                    tracing::warn!(
                        memory_id = %memory_id,
                        error = %e,
                        "Failed to insert quality embedding — skipping"
                    );
                    failed_count += 1;
                    continue;
                }

                tracing::debug!(
                    memory_id = %memory_id,
                    from_tier = source_tier,
                    to_tier = target_tier,
                    "Memory promoted to quality tier"
                );
                promoted_count += 1;
            }
            Err(e) => {
                tracing::warn!(
                    memory_id = %memory_id,
                    error = %e,
                    "Quality embedding failed — skipping promotion"
                );
                failed_count += 1;
            }
        }
    }

    metrics::counter!("memcp_promotion_sweeps_total").increment(1);
    if promoted_count > 0 {
        metrics::counter!("memcp_promotion_promoted_total").increment(promoted_count as u64);
    }

    Ok(PromotionResult {
        promoted_count,
        candidates_evaluated,
        failed_count,
        skipped_reason: None,
    })
}
