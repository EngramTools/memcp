/// Auto-store sidecar — automatic memory ingestion from conversation logs.
///
/// Watches configured log files, parses new entries, filters for relevance,
/// deduplicates, and stores as memories. Runs as a background tokio task
/// in the main server process.

pub mod filter;
pub mod parser;
pub mod watcher;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

use crate::config::AutoStoreConfig;
use crate::embedding::{EmbeddingJob, build_embedding_text};
use crate::embedding::pipeline::EmbeddingPipeline;
use crate::extraction::ExtractionJob;
use crate::extraction::pipeline::ExtractionPipeline;
use crate::store::{CreateMemory, MemoryStore};
use crate::store::postgres::PostgresMemoryStore;

use self::filter::{FilterStrategy, create_filter};
use self::parser::{LogParser, create_parser};
use self::watcher::{WatchEvent, spawn_watcher};

/// Background worker that watches log files and auto-stores memories.
pub struct AutoStoreWorker;

impl AutoStoreWorker {
    /// Spawn the auto-store background task.
    ///
    /// Returns a JoinHandle for the background task (runs until the server shuts down).
    /// The worker:
    /// 1. Watches configured files for new lines
    /// 2. Parses each line with the configured format parser
    /// 3. Deduplicates within a sliding time window
    /// 4. Filters for relevance (LLM, heuristic, or none)
    /// 5. Stores as a memory with type_hint="auto" and source from the parser
    /// 6. Enqueues to embedding and extraction pipelines
    pub fn spawn(
        config: AutoStoreConfig,
        store: Arc<PostgresMemoryStore>,
        embedding_pipeline: Option<&EmbeddingPipeline>,
        extraction_pipeline: Option<&ExtractionPipeline>,
        extraction_config: &crate::config::ExtractionConfig,
    ) -> JoinHandle<()> {
        let parser = create_parser(&config.format);
        let filter = create_filter(
            &config.filter_mode,
            &config.filter_provider,
            &config.filter_model,
            extraction_config,
        );
        let dedup_window = Duration::from_secs(config.dedup_window_secs);
        let poll_interval = Duration::from_secs(config.poll_interval_secs);

        // Clone senders from pipelines so we can move them into the task
        let embedding_sender = embedding_pipeline.map(|p| p.sender());
        let extraction_sender = extraction_pipeline.map(|p| p.sender());

        let watch_paths = config.watch_paths.clone();

        tracing::info!(
            paths = ?watch_paths,
            format = %config.format,
            filter_mode = %config.filter_mode,
            poll_interval_secs = config.poll_interval_secs,
            dedup_window_secs = config.dedup_window_secs,
            "Auto-store sidecar starting"
        );

        tokio::spawn(async move {
            run_worker(
                watch_paths,
                poll_interval,
                parser,
                filter,
                dedup_window,
                store,
                embedding_sender,
                extraction_sender,
            )
            .await;
        })
    }
}

/// Content hash for deduplication — uses the raw content string hash.
fn content_hash(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Main worker loop.
async fn run_worker(
    watch_paths: Vec<String>,
    poll_interval: Duration,
    parser: Box<dyn LogParser>,
    filter: Box<dyn FilterStrategy>,
    dedup_window: Duration,
    store: Arc<PostgresMemoryStore>,
    embedding_sender: Option<tokio::sync::mpsc::Sender<EmbeddingJob>>,
    extraction_sender: Option<tokio::sync::mpsc::Sender<ExtractionJob>>,
) {
    // Channel for watch events
    let (tx, mut rx) = tokio::sync::mpsc::channel::<WatchEvent>(1000);

    // Spawn the file watcher
    let _watcher_handle = spawn_watcher(watch_paths, poll_interval, tx);

    // Dedup sliding window: hash → last seen time
    let mut dedup_map: HashMap<u64, Instant> = HashMap::new();
    let mut last_dedup_cleanup = Instant::now();

    while let Some(event) = rx.recv().await {
        // Parse the line
        let entry = match parser.parse_line(&event.line) {
            Some(e) => e,
            None => continue,
        };

        // Dedup check
        let hash = content_hash(&entry.content);
        let now = Instant::now();
        if let Some(last_seen) = dedup_map.get(&hash) {
            if now.duration_since(*last_seen) < dedup_window {
                tracing::debug!(
                    content_preview = %entry.content.chars().take(50).collect::<String>(),
                    "Auto-store dedup: skipping duplicate within window"
                );
                continue;
            }
        }
        dedup_map.insert(hash, now);

        // Periodic cleanup of expired dedup entries
        if now.duration_since(last_dedup_cleanup) > dedup_window * 2 {
            dedup_map.retain(|_, seen| now.duration_since(*seen) < dedup_window);
            last_dedup_cleanup = now;
        }

        // Filter check
        match filter.should_store(&entry).await {
            Ok(true) => {} // proceed
            Ok(false) => {
                tracing::debug!(
                    content_preview = %entry.content.chars().take(50).collect::<String>(),
                    "Auto-store filter: skipping non-relevant entry"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Auto-store filter error, storing anyway");
                // On filter error, default to storing (fail open)
            }
        }

        // Build tags
        let mut tags = vec!["auto-store".to_string()];
        if let Some(ref sid) = entry.session_id {
            tags.push(format!("session:{}", sid));
        }
        if let Some(ref project) = entry.project {
            tags.push(format!("project:{}", project));
        }

        // Store the memory
        let create = CreateMemory {
            content: entry.content.clone(),
            type_hint: "auto".to_string(),
            source: entry.source.clone(),
            tags: Some(tags.clone()),
            created_at: entry.timestamp,
            actor: None,
            actor_type: "auto-store".to_string(),
            audience: "global".to_string(),
        };

        match store.store(create).await {
            Ok(memory) => {
                tracing::info!(
                    memory_id = %memory.id,
                    source = %entry.source,
                    content_len = entry.content.len(),
                    "Auto-stored memory"
                );

                // Enqueue to embedding pipeline
                if let Some(ref sender) = embedding_sender {
                    let text = build_embedding_text(&memory.content, &memory.tags);
                    let _ = sender.try_send(EmbeddingJob {
                        memory_id: memory.id.clone(),
                        text,
                        attempt: 0,
                    });
                }

                // Enqueue to extraction pipeline
                if let Some(ref sender) = extraction_sender {
                    let _ = sender.try_send(ExtractionJob {
                        memory_id: memory.id.clone(),
                        content: memory.content.clone(),
                        attempt: 0,
                    });
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    content_preview = %entry.content.chars().take(50).collect::<String>(),
                    "Failed to auto-store memory"
                );
            }
        }
    }

    tracing::warn!("Auto-store worker: watch event channel closed, shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        let h3 = content_hash("different content");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
