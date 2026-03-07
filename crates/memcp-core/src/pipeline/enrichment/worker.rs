//! Enrichment worker — periodic sweep to enrich un-enriched memories with neighbor-derived tags.
//!
//! Orchestrates: candidate fetch -> embedding lookup -> neighbor search -> LLM tag suggestion
//! -> tag application -> 'enriched' provenance marker.
//!
//! Mirrors the curation worker pattern (periodic interval, shutdown signal, fail-open per memory).

use std::sync::Arc;

use metrics;

use super::{EnrichmentProvider, EnrichmentError};
use crate::config::EnrichmentConfig;
use crate::consolidation::similarity::find_similar_memories;
use crate::errors::MemcpError;
use crate::store::postgres::PostgresMemoryStore;

/// Run the enrichment daemon worker.
///
/// Loops on a configurable interval, running sweeps to find and enrich
/// un-enriched memories. Stops cleanly when the shutdown channel fires.
///
/// Fail-open: individual memory failures are logged and skipped,
/// not propagated to the caller.
pub async fn run_enrichment(
    store: Arc<PostgresMemoryStore>,
    provider: Arc<dyn EnrichmentProvider>,
    config: EnrichmentConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), MemcpError> {
    let mut interval = tokio::time::interval(
        std::time::Duration::from_secs(config.sweep_interval_secs)
    );

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if *shutdown.borrow() {
                    break;
                }
                match run_enrichment_sweep(&store, &*provider, &config).await {
                    Ok(count) if count > 0 => {
                        tracing::info!(enriched = count, "Enrichment sweep complete");
                    }
                    Ok(_) => {
                        tracing::debug!("Enrichment sweep complete — no memories to enrich");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Enrichment sweep failed");
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Run a single enrichment sweep.
///
/// Fetches un-enriched memories, finds neighbors, calls LLM for tags, applies tags.
/// Returns the number of memories successfully enriched in this sweep.
async fn run_enrichment_sweep(
    store: &PostgresMemoryStore,
    provider: &dyn EnrichmentProvider,
    config: &EnrichmentConfig,
) -> Result<usize, MemcpError> {
    // 1. Fetch candidates — memories without the 'enriched' tag
    let candidates = store.get_unenriched_memories(config.batch_limit as i64).await?;

    if candidates.is_empty() {
        return Ok(0);
    }

    let valid_tag_re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$")
        .expect("enrichment tag regex is valid");

    let mut enriched_count = 0usize;

    for memory in &candidates {
        // 2. Get memory's embedding
        let embedding = match store.get_memory_embedding(&memory.id).await? {
            Some(e) => e,
            None => {
                // No embedding yet — memory hasn't been processed by embedding pipeline
                tracing::debug!(memory_id = %memory.id, "Skipping enrichment — no embedding yet");
                continue;
            }
        };

        // 3. Find nearest neighbors using pgvector similarity
        let neighbors = find_similar_memories(
            store.pool(),
            &memory.id,
            &embedding,
            config.neighbor_similarity_threshold,
            config.neighbor_depth as i64,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(memory_id = %memory.id, error = %e, "Neighbor search failed, using empty list");
            Vec::new()
        });

        if neighbors.is_empty() {
            // No neighbors above threshold — mark as enriched to avoid re-scanning
            store.add_memory_tag(&memory.id, "enriched").await
                .unwrap_or_else(|e| {
                    tracing::warn!(memory_id = %memory.id, error = %e, "Failed to mark enriched");
                });
            enriched_count += 1;
            continue;
        }

        // 4. Collect neighbor content strings
        let neighbor_contents: Vec<String> = neighbors.iter()
            .map(|n| n.content.clone())
            .collect();

        // 5. Call LLM for tag suggestions (fail-open per memory)
        let result = match provider.suggest_tags(&memory.content, &neighbor_contents).await {
            Ok(r) => r,
            Err(EnrichmentError::LlmUnavailable(e)) => {
                tracing::warn!(
                    memory_id = %memory.id,
                    error = %e,
                    "Enrichment LLM unavailable — skipping memory"
                );
                continue; // Don't mark enriched — retry next sweep when LLM is back
            }
            Err(EnrichmentError::ProviderError(e)) => {
                tracing::warn!(
                    memory_id = %memory.id,
                    error = %e,
                    "Enrichment LLM parse error — skipping memory"
                );
                continue;
            }
        };

        // 6. Apply sanitized tags
        for tag in &result.tags_to_add {
            let tag = tag.trim().to_lowercase();
            if tag.is_empty() || tag.len() > 50 {
                continue;
            }
            if !valid_tag_re.is_match(&tag) {
                tracing::debug!(tag = %tag, "Skipping tag with invalid characters");
                continue;
            }
            if let Err(e) = store.add_memory_tag(&memory.id, &tag).await {
                tracing::warn!(memory_id = %memory.id, tag = %tag, error = %e, "Failed to add tag");
            }
        }

        // 7. Mark as enriched (provenance marker, prevents re-processing)
        if let Err(e) = store.add_memory_tag(&memory.id, "enriched").await {
            tracing::warn!(memory_id = %memory.id, error = %e, "Failed to mark memory as enriched");
        }

        enriched_count += 1;
    }

    metrics::counter!("memcp_enrichment_sweeps_total").increment(1);
    if enriched_count > 0 {
        metrics::counter!("memcp_enrichment_memories_total").increment(enriched_count as u64);
    }

    Ok(enriched_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_tag_filter() {
        let re = regex::Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();

        // Valid tags
        assert!(re.is_match("rust-async"));
        assert!(re.is_match("tokio_runtime"));
        assert!(re.is_match("abc123"));
        assert!(re.is_match("my-tag"));

        // Invalid tags
        assert!(!re.is_match("tag with spaces"));
        assert!(!re.is_match("tag!"));
        assert!(!re.is_match("tag.dot"));
        assert!(!re.is_match(""));
        assert!(!re.is_match("tag/slash"));
    }

    #[test]
    fn test_tag_length_limit() {
        // Tags longer than 50 chars should be rejected
        let long_tag = "a".repeat(51);
        assert!(long_tag.len() > 50);

        let valid_tag = "a".repeat(50);
        assert_eq!(valid_tag.len(), 50);
    }
}
