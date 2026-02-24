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
        let result = find_similar_memories(
            self.store.pool(),
            &job.memory_id,
            &job.embedding,
            self.config.similarity_threshold,
            1,
        )
        .await;

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
                let best = &candidates[0];
                let existing_id = best.memory_id.clone();
                let similarity = best.similarity;

                let source_info = job.memory_id.as_str();

                match self.store.merge_duplicate(&existing_id, &job.memory_id, source_info).await {
                    Ok(()) => {
                        // Update dedup merge count metric
                        if let Err(e) = self.store.increment_dedup_merges().await {
                            tracing::warn!(error = %e, "Failed to increment dedup merge counter");
                        }
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
}
