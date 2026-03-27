//! Async embedding pipeline with bounded mpsc channel and background worker.
//!
//! Non-blocking design: store_memory never waits for embedding completion.
//! Failed embeddings are retried up to 3 times with exponential backoff (1s, 2s, 4s),
//! then marked as failed for backfill on next startup.
//!
//! Supports multi-tier embedding via EmbeddingRouter: each job specifies its target
//! tier, and the worker uses the corresponding provider for that tier.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::router::EmbeddingRouter;
use super::{build_embedding_text, EmbeddingCompletion, EmbeddingJob, EmbeddingProvider};
use crate::consolidation::ConsolidationJob;
use crate::gc::DedupJob;
use crate::store::postgres::PostgresMemoryStore;
use crate::store::MemoryStore;

/// Async embedding pipeline: enqueues jobs onto a bounded mpsc channel and
/// processes them in a background tokio task.
pub struct EmbeddingPipeline {
    sender: mpsc::Sender<EmbeddingJob>,
    /// Count of jobs currently in-flight (enqueued but not yet completed).
    /// Used by flush() to block until the pipeline drains.
    pending_count: Arc<AtomicUsize>,
}

impl EmbeddingPipeline {
    /// Create a new EmbeddingPipeline with an EmbeddingRouter for multi-tier support.
    ///
    /// - `router`: Routes each job to the correct embedding provider based on tier.
    /// - `store`: The PostgresMemoryStore for storing embeddings and updating status.
    /// - `capacity`: Bounded channel capacity (recommended: 1000).
    /// - `consolidation_sender`: Optional channel to the consolidation worker.
    /// - `dedup_sender`: Optional channel to the dedup worker.
    pub fn new(
        router: Arc<EmbeddingRouter>,
        store: Arc<PostgresMemoryStore>,
        capacity: usize,
        consolidation_sender: Option<mpsc::Sender<ConsolidationJob>>,
        dedup_sender: Option<mpsc::Sender<DedupJob>>,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<EmbeddingJob>(capacity);
        let retry_tx = tx.clone();

        let pending_count = Arc::new(AtomicUsize::new(0));
        let worker_pending = Arc::clone(&pending_count);

        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                let text = job.text.clone();
                let tier = job.tier.clone();

                // Select the provider for this job's tier, falling back to default
                let provider = router
                    .provider(&tier)
                    .unwrap_or_else(|| router.default_provider());

                let embed_start = std::time::Instant::now();
                match provider.embed(&text).await {
                    Ok(vector) => {
                        let duration = embed_start.elapsed().as_secs_f64();
                        let embedding = pgvector::Vector::from(vector);
                        let emb_id = Uuid::new_v4().to_string();
                        let model = provider.model_name().to_string();
                        let dim = provider.dimension() as i32;
                        if let Err(e) = store
                            .insert_embedding(
                                &emb_id,
                                &job.memory_id,
                                &model,
                                "v1",
                                dim,
                                &embedding,
                                true,
                                &tier,
                            )
                            .await
                        {
                            tracing::error!(
                                memory_id = %job.memory_id,
                                tier = %tier,
                                error = %e,
                                "Failed to store embedding"
                            );
                            let _ = store
                                .update_embedding_status(&job.memory_id, "failed")
                                .await;
                            metrics::counter!("memcp_embedding_jobs_total", "status" => "error")
                                .increment(1);
                            if let Some(tx) = job.completion_tx {
                                let _ = tx.send(EmbeddingCompletion {
                                    status: "failed".to_string(),
                                    dimension: None,
                                    tier: tier.clone(),
                                });
                            }
                            worker_pending.fetch_sub(1, Ordering::Relaxed);
                        } else {
                            let _ = store
                                .update_embedding_status(&job.memory_id, "complete")
                                .await;
                            metrics::counter!("memcp_embedding_jobs_total", "status" => "success")
                                .increment(1);
                            metrics::histogram!("memcp_embedding_duration_seconds", "tier" => tier.clone()).record(duration);
                            tracing::debug!(memory_id = %job.memory_id, tier = %tier, "Embedding complete");
                            if let Some(tx) = job.completion_tx {
                                let _ = tx.send(EmbeddingCompletion {
                                    status: "completed".to_string(),
                                    dimension: Some(dim),
                                    tier: tier.clone(),
                                });
                            }

                            // Update memory count gauges after each successful embedding
                            if let Ok(total) = store.count_live_memories().await {
                                metrics::gauge!("memcp_memories_total").set(total as f64);
                            }
                            if let Ok(pending) = store.count_pending_embeddings().await {
                                metrics::gauge!("memcp_memories_pending_embedding")
                                    .set(pending as f64);
                            }

                            // Trigger consolidation check
                            if let Some(ref consolidation_tx) = consolidation_sender {
                                match store.get(&job.memory_id).await {
                                    Ok(memory) => {
                                        let _ = consolidation_tx.try_send(ConsolidationJob {
                                            memory_id: job.memory_id.clone(),
                                            embedding: embedding.clone(),
                                            content: memory.content,
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            memory_id = %job.memory_id,
                                            error = %e,
                                            "Failed to fetch memory for consolidation job — skipping"
                                        );
                                    }
                                }
                            }

                            // Trigger dedup check
                            if let Some(ref dedup_tx) = dedup_sender {
                                let _ = dedup_tx.try_send(DedupJob {
                                    memory_id: job.memory_id.clone(),
                                    embedding: embedding.clone(),
                                });
                            }

                            worker_pending.fetch_sub(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) if job.attempt < 3 => {
                        tracing::warn!(
                            memory_id = %job.memory_id,
                            tier = %tier,
                            attempt = job.attempt + 1,
                            error = %e,
                            "Embedding failed, retrying"
                        );
                        let delay = Duration::from_secs(2u64.pow(job.attempt as u32));
                        tokio::time::sleep(delay).await;
                        let _ = retry_tx.try_send(EmbeddingJob {
                            memory_id: job.memory_id,
                            text: job.text,
                            attempt: job.attempt + 1,
                            completion_tx: job.completion_tx,
                            tier,
                        });
                    }
                    Err(e) => {
                        tracing::error!(
                            memory_id = %job.memory_id,
                            tier = %tier,
                            attempts = 3,
                            error = %e,
                            "Embedding failed after 3 retries, marking as failed"
                        );
                        let _ = store
                            .update_embedding_status(&job.memory_id, "failed")
                            .await;
                        metrics::counter!("memcp_embedding_jobs_total", "status" => "error")
                            .increment(1);
                        if let Some(tx) = job.completion_tx {
                            let _ = tx.send(EmbeddingCompletion {
                                status: "failed".to_string(),
                                dimension: None,
                                tier,
                            });
                        }
                        worker_pending.fetch_sub(1, Ordering::Relaxed);
                    }
                }
            }
        });

        EmbeddingPipeline {
            sender: tx,
            pending_count,
        }
    }

    /// Create a pipeline with a single embedding provider (backward compatibility).
    ///
    /// Wraps the provider in a single-tier EmbeddingRouter internally.
    /// Used by MCP serve mode and tests that don't need multi-tier routing.
    pub fn new_single(
        provider: Arc<dyn EmbeddingProvider + Send + Sync>,
        store: Arc<PostgresMemoryStore>,
        capacity: usize,
        consolidation_sender: Option<mpsc::Sender<ConsolidationJob>>,
        dedup_sender: Option<mpsc::Sender<DedupJob>>,
    ) -> Self {
        let mut tiers = std::collections::HashMap::new();
        tiers.insert("fast".to_string(), (provider, None));
        let router = Arc::new(EmbeddingRouter::new(tiers, "fast".to_string()));
        Self::new(router, store, capacity, consolidation_sender, dedup_sender)
    }

    /// Enqueue an embedding job (non-blocking).
    ///
    /// Uses try_send — if the channel is full, the job is dropped and a warning is logged.
    /// The backfill process will pick up missed memories on next startup.
    pub fn enqueue(&self, job: EmbeddingJob) {
        self.pending_count.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = self.sender.try_send(job) {
            self.pending_count.fetch_sub(1, Ordering::Relaxed);
            let inner = e.into_inner();
            let tier = inner.tier;
            if let Some(tx) = inner.completion_tx {
                let _ = tx.send(EmbeddingCompletion {
                    status: "pending".to_string(),
                    dimension: None,
                    tier,
                });
            }
            tracing::warn!("Embedding queue full — memory stored, embedding deferred to backfill");
        }
    }

    /// Return a clone of the underlying mpsc sender (for use with the backfill function).
    pub fn sender(&self) -> mpsc::Sender<EmbeddingJob> {
        self.sender.clone()
    }

    /// Wait until all enqueued embedding jobs have completed (success or failure).
    /// Polls pending count every 100ms. Used by benchmark to ensure all embeddings
    /// are complete before running search.
    pub async fn flush(&self) {
        loop {
            let pending = self.pending_count.load(Ordering::Relaxed);
            if pending == 0 {
                break;
            }
            tracing::debug!(pending, "Waiting for embedding pipeline to flush");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

/// Queue all pending/failed memories for re-embedding.
///
/// Queries the store in batches of 100 and enqueues each memory on the pipeline channel.
/// Returns the total count of memories queued.
pub async fn backfill(store: &PostgresMemoryStore, sender: &mpsc::Sender<EmbeddingJob>) -> u64 {
    let mut total_queued: u64 = 0;

    loop {
        let pending = match store.get_pending_memories(100).await {
            Ok(memories) => memories,
            Err(e) => {
                tracing::error!(error = %e, "Failed to fetch pending memories for backfill");
                break;
            }
        };

        if pending.is_empty() {
            break;
        }

        let batch_size = pending.len() as u64;
        for memory in pending {
            let text = build_embedding_text(
                &memory.content,
                memory.abstract_text.as_deref(),
                &memory.tags,
            );
            let job = EmbeddingJob {
                memory_id: memory.id,
                text,
                attempt: 0,
                completion_tx: None,
                tier: "fast".to_string(),
            };
            if sender.try_send(job).is_err() {
                tracing::warn!("Embedding queue full during backfill — some memories deferred");
                return total_queued;
            }
        }

        total_queued += batch_size;

        if batch_size < 100 {
            break;
        }
    }

    if total_queued > 0 {
        tracing::info!(
            count = total_queued,
            "Queued memories for embedding backfill"
        );
    }

    total_queued
}
