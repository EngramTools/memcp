//! Auto-store sidecar — watches JSONL session files and ingests memories.
//!
//! Parses Claude Code / OpenClaw session logs, applies category filtering,
//! optional summarization, and stores memories via storage/. Watches directories
//! for new files. Feeds from pipeline/content_filter/ and pipeline/summarization/.

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

use crate::config::{AutoStoreConfig, ChunkingConfig};
use crate::chunking::chunk_content;
use crate::content_filter::{ContentFilter, FilterVerdict};
use crate::embedding::{EmbeddingJob, build_embedding_text};
use crate::embedding::pipeline::EmbeddingPipeline;
use crate::extraction::ExtractionJob;
use crate::extraction::pipeline::ExtractionPipeline;
use crate::store::{CreateMemory, MemoryStore};
use crate::store::postgres::PostgresMemoryStore;
use crate::summarization::SummarizationProvider;

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
        chunking_config: ChunkingConfig,
        store: Arc<PostgresMemoryStore>,
        embedding_pipeline: Option<&EmbeddingPipeline>,
        extraction_pipeline: Option<&ExtractionPipeline>,
        extraction_config: &crate::config::ExtractionConfig,
        content_filter: Option<Arc<dyn ContentFilter>>,
        summarization_provider: Option<Arc<dyn SummarizationProvider>>,
    ) -> JoinHandle<()> {
        let parser = create_parser(&config.format);
        let filter = create_filter(
            &config.filter_mode,
            &config.filter_provider,
            &config.filter_model,
            extraction_config,
            &config,
        );
        let dedup_window = Duration::from_secs(config.dedup_window_secs);
        let poll_interval = Duration::from_secs(config.poll_interval_secs);

        // Clone senders from pipelines so we can move them into the task
        let embedding_sender = embedding_pipeline.map(|p| p.sender());
        let extraction_sender = extraction_pipeline.map(|p| p.sender());

        let mut watch_paths = config.watch_paths.clone();

        // Auto-discover Claude Code JSONL directory if no watch paths configured
        if watch_paths.is_empty() {
            if let Some(home) = dirs::home_dir() {
                let claude_dir = home.join(".claude").join("projects");
                if claude_dir.exists() {
                    tracing::info!(
                        path = %claude_dir.display(),
                        "Auto-discovered Claude Code projects directory"
                    );
                    watch_paths.push(claude_dir.to_string_lossy().to_string());
                }
            }
        }

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
                content_filter,
                summarization_provider,
                chunking_config,
            )
            .await;
        })
    }
}

/// Content hash for deduplication — uses the raw content string hash.
/// FNV-1a content hash. Exposed as `pub` for external test access.
pub fn content_hash(content: &str) -> u64 {
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
    content_filter: Option<Arc<dyn ContentFilter>>,
    summarization_provider: Option<Arc<dyn SummarizationProvider>>,
    chunking_config: ChunkingConfig,
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
        let entry = match parser.parse_line(&event.line, &event.path) {
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

        // Content exclusion filter (BEFORE relevance filter — cheaper and deterministic)
        if let Some(ref cf) = content_filter {
            match cf.check(&entry.content).await {
                Ok(FilterVerdict::Drop { reason }) => {
                    tracing::debug!(
                        reason = %reason,
                        content_preview = %entry.content.chars().take(50).collect::<String>(),
                        "Auto-store: content excluded by filter"
                    );
                    continue;
                }
                Ok(FilterVerdict::Allow) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "Auto-store: content filter error, proceeding");
                }
            }
        }

        // Filter check
        let category_result = match filter.should_store(&entry).await {
            Ok(true) => {
                // Get classification result if available (CategoryFilter with LLM)
                filter.last_classification()
            }
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
                None
            }
        };

        // Summarize assistant responses, store user messages raw
        let is_assistant = entry.metadata.get("role").map(|r| r == "assistant").unwrap_or(false);
        let (store_content, is_summarized) = if is_assistant {
            if let Some(ref provider) = summarization_provider {
                match provider.summarize(&entry.content).await {
                    Ok(summary) => {
                        tracing::debug!(
                            original_len = entry.content.len(),
                            summary_len = summary.len(),
                            "Auto-store: summarized assistant response"
                        );
                        (summary, true)
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            content_preview = %entry.content.chars().take(50).collect::<String>(),
                            "Auto-store: summarization failed, storing raw (fail-open)"
                        );
                        (entry.content.clone(), false)
                    }
                }
            } else {
                (entry.content.clone(), false)
            }
        } else {
            (entry.content.clone(), false)
        };

        // Build tags
        let mut tags = vec!["auto-store".to_string()];
        if is_summarized {
            tags.push("summarized".to_string());
        }
        if let Some(ref sid) = entry.session_id {
            tags.push(format!("session:{}", sid));
        }
        if let Some(ref project) = entry.project {
            tags.push(format!("project:{}", project));
        }
        // Add category tag from LLM classification
        if let Some(ref cr) = category_result {
            tags.push(format!("category:{}", cr.category));
        }

        // Store the memory
        let create = CreateMemory {
            content: store_content,
            type_hint: if is_summarized { "summary".to_string() } else { "auto".to_string() },
            source: entry.source.clone(),
            tags: Some(tags.clone()),
            created_at: entry.timestamp,
            actor: entry.actor.clone(),
            actor_type: "auto-store".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
        };

        match store.store(create).await {
            Ok(memory) => {
                // Seed salience: auto-store gets stability=2.5 (weaker than explicit store's 3.0)
                // "store-low" categories get stability=1.5 (even weaker — ephemeral-ish content)
                let stability = match &category_result {
                    Some(cr) if cr.action == "store-low" => 1.5,
                    _ => 2.5,
                };
                if let Err(e) = store.upsert_salience(&memory.id, stability, 5.0, 0, None).await {
                    tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience for auto-store");
                }

                // Update sidecar ingest tracking in daemon_status
                let today = chrono::Utc::now().date_naive();
                if let Err(e) = sqlx::query(
                    "UPDATE daemon_status SET \
                         last_ingest_at = NOW(), \
                         ingest_count_today = CASE \
                             WHEN ingest_date = $1 THEN ingest_count_today + 1 \
                             ELSE 1 \
                         END, \
                         ingest_date = $1 \
                     WHERE id = 1"
                )
                .bind(today)
                .execute(store.pool())
                .await
                {
                    tracing::warn!(error = %e, "Failed to update ingest tracking");
                }

                tracing::info!(
                    memory_id = %memory.id,
                    source = %entry.source,
                    content_len = memory.content.len(),
                    summarized = is_summarized,
                    "Auto-stored memory"
                );

                // Enqueue to embedding pipeline
                if let Some(ref sender) = embedding_sender {
                    let text = build_embedding_text(&memory.content, &memory.tags);
                    let _ = sender.try_send(EmbeddingJob {
                        memory_id: memory.id.clone(),
                        text,
                        attempt: 0,
                        completion_tx: None,
                        tier: "fast".to_string(),
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

                // Chunk long content for better retrieval granularity
                let chunks = chunk_content(&memory.content, &chunking_config);
                if !chunks.is_empty() {
                    tracing::info!(
                        memory_id = %memory.id,
                        chunk_count = chunks.len(),
                        "Chunking auto-store content"
                    );

                    for chunk_output in &chunks {
                        let mut chunk_tags = tags.clone();
                        chunk_tags.push(format!("chunk:{}/{}", chunk_output.index + 1, chunk_output.total));

                        let chunk_create = CreateMemory {
                            content: chunk_output.content.clone(),
                            type_hint: if is_summarized { "summary".to_string() } else { "auto".to_string() },
                            source: entry.source.clone(),
                            tags: Some(chunk_tags),
                            created_at: entry.timestamp,
                            actor: entry.actor.clone(),
                            actor_type: "auto-store".to_string(),
                            audience: "global".to_string(),
                            idempotency_key: None,
                            parent_id: Some(memory.id.clone()),
                            chunk_index: Some(chunk_output.index as i32),
                            total_chunks: Some(chunk_output.total as i32),
                        };

                        match store.store(chunk_create).await {
                            Ok(chunk_mem) => {
                                // Seed chunk salience from parent values
                                if let Err(e) = store.upsert_salience(&chunk_mem.id, 2.5, 5.0, 0, None).await {
                                    tracing::warn!(error = %e, chunk_id = %chunk_mem.id, "Failed to seed chunk salience");
                                }

                                // Enqueue chunk to embedding pipeline
                                if let Some(ref sender) = embedding_sender {
                                    let text = build_embedding_text(&chunk_mem.content, &chunk_mem.tags);
                                    let _ = sender.try_send(EmbeddingJob {
                                        memory_id: chunk_mem.id.clone(),
                                        text,
                                        attempt: 0,
                                        completion_tx: None,
                                        tier: "fast".to_string(),
                                    });
                                }

                                // Enqueue chunk to extraction pipeline
                                if let Some(ref sender) = extraction_sender {
                                    let _ = sender.try_send(ExtractionJob {
                                        memory_id: chunk_mem.id.clone(),
                                        content: chunk_mem.content.clone(),
                                        attempt: 0,
                                    });
                                }

                                tracing::debug!(
                                    chunk_id = %chunk_mem.id,
                                    parent_id = %memory.id,
                                    index = chunk_output.index,
                                    total = chunk_output.total,
                                    "Stored chunk"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    parent_id = %memory.id,
                                    chunk_index = chunk_output.index,
                                    "Failed to store chunk"
                                );
                            }
                        }
                    }
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

