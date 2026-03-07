/// Async semantic deduplication worker.
///
/// Runs post-embedding to detect and merge near-duplicate memories.
/// Near-duplicates (cosine similarity >= threshold) are merged:
///   - Existing memory metadata updated (access_count, last_accessed_at, dedup_sources)
///   - New (incoming) memory soft-deleted
///   - Merge tracked in daemon_status.gc_dedup_merges
///
/// Fail-open: errors are logged, never cause data loss.
/// Zero ingest latency impact — all processing is fully async.

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::DedupConfig;
use crate::consolidation::similarity::find_similar_memories;
use crate::store::MemoryStore;
use crate::store::postgres::PostgresMemoryStore;

/// A job sent to the dedup worker after a memory's embedding is complete.
pub struct DedupJob {
    /// The ID of the newly embedded memory to check for duplicates.
    pub memory_id: String,
    /// The embedding vector for similarity comparison.
    pub embedding: pgvector::Vector,
}

/// Background worker that consumes DedupJobs and merges near-duplicates.
pub struct DedupWorker {
    store: Arc<PostgresMemoryStore>,
    config: DedupConfig,
    receiver: mpsc::Receiver<DedupJob>,
}

impl DedupWorker {
    /// Create a new DedupWorker.
    ///
    /// - `store`: The PostgresMemoryStore for similarity checks and merge operations.
    /// - `config`: DedupConfig controlling the similarity threshold and enabled flag.
    /// - `receiver`: mpsc channel receiver for incoming DedupJobs.
    pub fn new(
        store: Arc<PostgresMemoryStore>,
        config: DedupConfig,
        receiver: mpsc::Receiver<DedupJob>,
    ) -> Self {
        DedupWorker { store, config, receiver }
    }

    /// Run the dedup event loop.
    ///
    /// Consumes DedupJobs from the mpsc receiver until the channel is closed (daemon shutdown).
    /// For each job:
    ///   1. Searches for the most similar existing memory above the configured threshold.
    ///   2. If found: merges by updating the existing memory and soft-deleting the new one.
    ///   3. If not found: memory is not a duplicate, passes through unaffected.
    pub async fn run(mut self) {
        while let Some(job) = self.receiver.recv().await {
            self.process_job(job).await;
        }
    }

    /// Process a single DedupJob.
    async fn process_job(&self, job: DedupJob) {
        // Fetch extra candidates so we can skip chunk siblings and still find a match
        let result = find_similar_memories(
            self.store.pool(),
            &job.memory_id,
            &job.embedding,
            self.config.similarity_threshold,
            5,
        )
        .await;

        // Look up parent_id of the incoming memory (for chunk-sibling detection)
        let job_parent_id: Option<String> = match self.store.get(&job.memory_id).await {
            Ok(m) => m.parent_id,
            Err(_) => None,
        };

        match result {
            Err(e) => {
                tracing::warn!(
                    memory_id = %job.memory_id,
                    error = %e,
                    "Dedup similarity search failed — skipping (fail-open)"
                );
            }
            Ok(ref candidates) if candidates.is_empty() => {
                // No duplicate found — memory passes through unaffected
                tracing::debug!(memory_id = %job.memory_id, "Dedup: no duplicate found");
            }
            Ok(candidates) => {
                // Skip chunk siblings: if the incoming memory is a chunk, ignore
                // candidates that share the same parent_id (they're slices of the same content).
                let (existing_id, similarity) = if let Some(pid) = &job_parent_id {
                    match self.find_non_sibling(&candidates, pid).await {
                        Some((id, sim)) => (id.clone(), sim),
                        None => {
                            tracing::debug!(
                                memory_id = %job.memory_id,
                                "Dedup: all candidates are chunk siblings — no duplicate"
                            );
                            return;
                        }
                    }
                } else {
                    (candidates[0].memory_id.clone(), candidates[0].similarity)
                };

                let source_info = job.memory_id.as_str();

                match self.store.merge_duplicate(&existing_id, &job.memory_id, source_info).await {
                    Ok(()) => {
                        // Update dedup merge count metric (DB-level) and Prometheus counter
                        if let Err(e) = self.store.increment_dedup_merges().await {
                            tracing::warn!(error = %e, "Failed to increment dedup merge counter");
                        }
                        metrics::counter!("memcp_dedup_merges_total").increment(1);
                        tracing::info!(
                            new_id = %job.memory_id,
                            existing_id = %existing_id,
                            similarity = format!("{:.3}", similarity),
                            "Dedup: merged memory into existing (near-duplicate detected)"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            memory_id = %job.memory_id,
                            existing_id = %existing_id,
                            error = %e,
                            "Dedup merge failed — skipping (fail-open)"
                        );
                    }
                }
            }
        }
    }

    /// Find the first candidate that is NOT a chunk sibling of the incoming memory.
    ///
    /// Two chunks are siblings if they share the same parent_id. We skip these because
    /// chunks from the same parent naturally have high similarity (overlapping content).
    async fn find_non_sibling<'a>(
        &self,
        candidates: &'a [crate::consolidation::similarity::SimilarMemory],
        job_parent_id: &str,
    ) -> Option<(&'a String, f64)> {
        for candidate in candidates {
            // Look up the candidate's parent_id
            match self.store.get(&candidate.memory_id).await {
                Ok(m) => {
                    if m.parent_id.as_deref() == Some(job_parent_id) {
                        // Same parent — skip this sibling
                        tracing::debug!(
                            candidate_id = %candidate.memory_id,
                            parent_id = %job_parent_id,
                            "Dedup: skipping chunk sibling"
                        );
                        continue;
                    }
                    return Some((&candidate.memory_id, candidate.similarity));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        candidate_id = %candidate.memory_id,
                        "Dedup: failed to fetch candidate for sibling check — skipping"
                    );
                    continue;
                }
            }
        }
        None
    }
}
