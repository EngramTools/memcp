//! Auto-store sidecar — watches JSONL session files and ingests memories.
//!
//! Parses Claude Code / OpenClaw session logs, applies category filtering,
//! optional summarization, and stores memories via storage/. Watches directories
//! for new files. Feeds from pipeline/content_filter/ and pipeline/summarization/.

pub mod filter;
pub mod parser;
pub mod shared;
pub mod watcher;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

use crate::config::AutoStoreConfig;
use crate::content_filter::ContentFilter;
use crate::embedding::pipeline::EmbeddingPipeline;
use crate::embedding::router::EmbeddingRouter;
use crate::embedding::{build_embedding_text, EmbeddingJob};
use crate::extraction::pipeline::ExtractionPipeline;
use crate::extraction::ExtractionJob;
use crate::pipeline::redaction::RedactionEngine;
use crate::store::postgres::PostgresMemoryStore;
use crate::store::StoreOutcome;
use crate::summarization::SummarizationProvider;

use self::filter::{create_filter, FilterStrategy};
use self::parser::{create_parser, LogParser};
use self::shared::{
    process_ingest_message, ProcessMessageContext, ProcessMessageInput, ProcessOutcome,
};
use self::watcher::{spawn_watcher, WatchEvent};

/// Bundled context for spawning the auto-store worker.
pub struct AutoStoreContext<'a> {
    // Config
    pub config: AutoStoreConfig,
    pub extraction_config: &'a crate::config::ExtractionConfig,
    // Store
    pub store: Arc<PostgresMemoryStore>,
    // Pipelines
    pub embedding_pipeline: Option<&'a EmbeddingPipeline>,
    pub extraction_pipeline: Option<&'a ExtractionPipeline>,
    pub embedding_router: Option<Arc<EmbeddingRouter>>,
    // Filters
    pub content_filter: Option<Arc<dyn ContentFilter>>,
    pub redaction_engine: Option<Arc<RedactionEngine>>,
    // Processing
    pub summarization_provider: Option<Arc<dyn SummarizationProvider>>,
    // Context
    pub project: Option<String>,
    pub birth_year: Option<u32>,
}

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
    pub fn spawn(ctx: AutoStoreContext<'_>) -> JoinHandle<()> {
        let AutoStoreContext {
            config,
            extraction_config,
            store,
            embedding_pipeline,
            extraction_pipeline,
            embedding_router,
            content_filter,
            redaction_engine,
            summarization_provider,
            project,
            birth_year,
        } = ctx;
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
                embedding_router,
                project,
                birth_year,
                redaction_engine,
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
#[allow(clippy::too_many_arguments)] // Internal function, not public API
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
    embedding_router: Option<Arc<EmbeddingRouter>>,
    project: Option<String>,
    birth_year: Option<u32>,
    redaction_engine: Option<Arc<RedactionEngine>>,
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

        // Filter check (auto-store-only; determines whether to ingest at all + supplies
        // the category classification consumed by the shared helper for salience seeding).
        let category_result = match filter.should_store(&entry).await {
            Ok(true) => filter.last_classification(),
            Ok(false) => {
                tracing::debug!(
                    content_preview = %entry.content.chars().take(50).collect::<String>(),
                    "Auto-store filter: skipping non-relevant entry"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Auto-store filter error, storing anyway");
                None
            }
        };

        let role = entry.metadata.get("role").cloned().unwrap_or_default();
        let role_for_helper = if role.is_empty() { "user" } else { role.as_str() };
        let session_id_str = entry.session_id.clone().unwrap_or_default();
        let project_str = entry
            .project
            .clone()
            .or_else(|| project.clone())
            .unwrap_or_default();

        let input = ProcessMessageInput {
            source: entry.source.as_str(),
            session_id: session_id_str.as_str(),
            project: project_str.as_str(),
            role: role_for_helper,
            content: entry.content.as_str(),
            timestamp: entry.timestamp,
            idempotency_key: None,
            reply_to_id: None,
            actor: entry.actor.clone(),
            write_path: "auto_store",
            base_tags: vec!["auto-store".to_string()],
            category: category_result.clone(),
            birth_year,
        };

        let helper_ctx = ProcessMessageContext {
            store: &store,
            redaction_engine: redaction_engine.as_deref(),
            content_filter: content_filter.as_ref(),
            summarization_provider: summarization_provider.as_ref(),
            embed_sender: embedding_sender.as_ref(),
            extract_sender: extraction_sender.as_ref(),
        };

        let helper_outcome = process_ingest_message(&helper_ctx, input).await;

        // Map the helper's outcome back onto the worker's original control flow so the
        // per-memory post-actions (companion .ids.jsonl, chunking, daemon_status tick)
        // keep firing exactly as before.
        let memory = match helper_outcome {
            ProcessOutcome::Stored {
                outcome: StoreOutcome::Created(memory),
                ..
            } => memory,
            ProcessOutcome::Stored {
                outcome: StoreOutcome::Deduplicated(_),
                ..
            } => {
                // Dedup hit — helper already logged it via storage layer. Skip companions.
                continue;
            }
            ProcessOutcome::Filtered { .. } => {
                // Helper logged the drop. Skip companions.
                continue;
            }
            ProcessOutcome::Errored { error } => {
                tracing::error!(
                    error = %error,
                    content_preview = %entry.content.chars().take(50).collect::<String>(),
                    "Failed to auto-store memory"
                );
                continue;
            }
        };

        // Tags used by the helper match what we used to build here; reconstruct the set
        // that the companion emission observed. (Helper tagging order: base, summarized?,
        // session:X, project:Y, category:Z.)
        let is_summarized = memory.type_hint == "summary";
        let mut tags = vec!["auto-store".to_string()];
        if is_summarized {
            tags.push("summarized".to_string());
        }
        tags.push(format!("session:{}", session_id_str));
        tags.push(format!("project:{}", project_str));
        if let Some(ref cr) = category_result {
            tags.push(format!("category:{}", cr.category));
        }

        // Derive companion file path for ID emission: conversation.jsonl -> conversation.ids.jsonl
        let companion_path = {
            let stem = event.path.file_stem().unwrap_or_default();
            event
                .path
                .with_file_name(format!("{}.ids.jsonl", stem.to_string_lossy()))
        };

        // Emit ID to companion .ids.jsonl file (fail-open)
        let emission = serde_json::json!({
            "memory_id": memory.id,
            "role": role,
            "tags": tags,
            "type_hint": memory.type_hint,
            "content_preview": memory.content.chars().take(100).collect::<String>(),
            "created_at": memory.created_at.to_rfc3339(),
            "source_line": event.line,
        });
        append_id_emission(&companion_path, &emission).await;

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
             WHERE id = 1",
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

        // Embedding tier override: the helper enqueues against "fast". If a multi-tier
        // router is configured, re-enqueue on the router-picked tier so the right model
        // picks up the job. Idempotent at the pipeline level (pending status already set).
        let type_hint_str = if is_summarized { "summary" } else { "auto" };
        if let Some(ref router) = embedding_router {
            let tier = router
                .route(Some(type_hint_str), None, memory.content.len())
                .to_string();
            if tier != "fast" {
                if let Some(ref sender) = embedding_sender {
                    let text = build_embedding_text(
                        &memory.content,
                        memory.abstract_text.as_deref(),
                        &memory.tags,
                    );
                    let _ = sender.try_send(EmbeddingJob {
                        memory_id: memory.id.clone(),
                        text,
                        attempt: 0,
                        completion_tx: None,
                        tier,
                    });
                }
            }
        }

        // Phase 24.75: chunking removed. Auto-store stores whole content as a single
        // memory row; precision on long content comes from Phase 27 (agentic retrieval)
        // and Phase 29 (multi-depth summaries). `get_memory_span` (Plan 24.75-04) does
        // on-demand splitting at query time using pipeline/chunking/splitter.rs.
    }

    tracing::warn!("Auto-store worker: watch event channel closed, shutting down");
}

/// Append a single JSONL line to the companion .ids.jsonl file.
///
/// Fail-open: any write failure logs a warning but does not stop processing.
async fn append_id_emission(path: &std::path::Path, payload: &serde_json::Value) {
    let line = match serde_json::to_string(payload) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to serialize ID emission");
            return;
        }
    };
    match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = file.write_all(format!("{}\n", line).as_bytes()).await {
                tracing::warn!(error = %e, path = %path.display(), "Failed to write ID emission to companion file");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "Failed to open companion file for ID emission");
        }
    }
}
