//! Daemon mode -- long-running process hosting background workers.
//!
//! Runs embedding pipeline, extraction pipeline, consolidation worker,
//! auto-store sidecar, and content filter as background tasks.
//! Writes heartbeat to `daemon_status` table every 30 seconds.
//! Polls for pending embedding/extraction work every 10 seconds.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use chrono::Utc;
use tokio::signal;

use crate::config::Config;
use crate::consolidation::ConsolidationWorker;
use crate::content_filter::CompositeFilter;
use crate::embedding::EmbeddingProvider;
#[cfg(feature = "local-embed")]
use crate::embedding::local::LocalEmbeddingProvider;
use crate::embedding::openai::OpenAIEmbeddingProvider;
use crate::embedding::pipeline::{EmbeddingPipeline, backfill};
use crate::extraction::{ExtractionJob, ExtractionProvider};
use crate::extraction::ollama::OllamaExtractionProvider;
use crate::extraction::openai::OpenAIExtractionProvider;
use crate::extraction::pipeline::ExtractionPipeline;
use crate::gc::{self, DedupWorker};
use crate::ipc::{embed_socket_path, start_embed_listener};
use crate::query_intelligence::QueryIntelligenceProvider;
use crate::query_intelligence::ollama::OllamaQueryIntelligenceProvider;
use crate::query_intelligence::openai::OpenAIQueryIntelligenceProvider;
use crate::store::postgres::PostgresMemoryStore;
use crate::summarization::create_summarization_provider;

/// Main daemon entry point.
///
/// Initializes all background workers, runs backfill, then enters
/// a heartbeat + poll loop until SIGINT/SIGTERM.
pub async fn run_daemon(config: &Config, skip_migrate: bool) -> Result<()> {
    let run_migrations = !skip_migrate;

    // 1. Initialize store
    let store = Arc::new(
        PostgresMemoryStore::new(&config.database_url, run_migrations)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize database: {}", e))?,
    );
    tracing::info!(database_url = %config.database_url, "PostgreSQL store initialized");

    // 2. Create embedding provider (loads fastembed model into memory)
    let provider = create_embedding_provider(config).await?;
    let provider_for_filter = provider.clone();

    // 2.5. Ensure HNSW index exists with the correct dimension for the active provider.
    // Migration 010 dropped the old vector(384)-typed HNSW index; we recreate it here
    // with a dimension-aware cast so the index works for any configured model.
    let dim = provider.dimension();
    store.ensure_hnsw_index(dim).await
        .map_err(|e| anyhow::anyhow!("Failed to ensure HNSW index: {}", e))?;
    tracing::info!(dimension = dim, "HNSW index ready");

    // 3. Create consolidation worker if enabled
    let consolidation_sender = if config.consolidation.enabled {
        let worker = ConsolidationWorker::new(
            store.clone(),
            config.consolidation.clone(),
            config.extraction.ollama_base_url.clone(),
            config.extraction.ollama_model.clone(),
            500,
        );
        tracing::info!(
            threshold = config.consolidation.similarity_threshold,
            max_group = config.consolidation.max_consolidation_group,
            "Consolidation worker started"
        );
        Some(worker.sender())
    } else {
        tracing::info!("Consolidation disabled via config");
        None
    };

    // 3.5. Create dedup worker if enabled
    let dedup_sender = if config.dedup.enabled {
        let (tx, rx) = tokio::sync::mpsc::channel(1000);
        let worker = DedupWorker::new(store.clone(), config.dedup.clone(), rx);
        tokio::spawn(async move { worker.run().await });
        tracing::info!(
            threshold = config.dedup.similarity_threshold,
            "Dedup worker started"
        );
        Some(tx)
    } else {
        tracing::info!("Dedup disabled via config");
        None
    };

    // 3.7. Spawn IPC listener so CLI can obtain embeddings and LLM re-ranking from the daemon.
    // The listener serves both embed and rerank requests from short-lived CLI processes over
    // a Unix domain socket, enabling full pipeline parity with MCP serve (SCF-01 gap closure).
    {
        let embed_provider = provider_for_filter.clone();
        let socket_path = embed_socket_path();

        // Create QI reranking provider if reranking is enabled.
        let qi_provider: Option<Arc<dyn QueryIntelligenceProvider + Send + Sync>> =
            if config.query_intelligence.reranking_enabled {
                match create_qi_reranking_provider(config) {
                    Ok(p) => {
                        tracing::info!("QI reranking provider available for IPC (CLI re-ranking)");
                        Some(p)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "QI reranking init failed — IPC rerank will no-op");
                        None
                    }
                }
            } else {
                tracing::debug!("QI reranking disabled — IPC rerank will no-op");
                None
            };

        tokio::spawn(async move {
            start_embed_listener(socket_path, embed_provider, qi_provider).await;
        });
    }

    // 4. Create embedding pipeline
    let pipeline = EmbeddingPipeline::new(provider, store.clone(), 1000, consolidation_sender, dedup_sender);

    // 5. Run startup embedding backfill
    let queued = backfill(&store, &pipeline.sender()).await;
    if queued > 0 {
        tracing::info!(count = queued, "Startup backfill queued memories for embedding");
    }

    // 6. Create extraction pipeline if enabled, backfill pending extractions
    let extraction_pipeline = if config.extraction.enabled {
        match create_extraction_provider(config) {
            Ok(extraction_provider) => {
                let ep = ExtractionPipeline::new(extraction_provider, store.clone(), 1000);
                match store.get_pending_extraction(1000).await {
                    Ok(pending) => {
                        let count = pending.len();
                        for (memory_id, content) in pending {
                            ep.enqueue(ExtractionJob {
                                memory_id,
                                content,
                                attempt: 0,
                            });
                        }
                        if count > 0 {
                            tracing::info!(count = count, "Startup backfill queued memories for extraction");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to fetch pending extractions for backfill");
                    }
                }
                Some(ep)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize extraction provider -- extraction disabled");
                None
            }
        }
    } else {
        tracing::info!("Extraction disabled via config");
        None
    };

    // 7. Build content filter if enabled
    let content_filter: Option<Arc<dyn crate::content_filter::ContentFilter>> =
        if config.content_filter.enabled {
            match CompositeFilter::from_config(
                &config.content_filter,
                Some(provider_for_filter),
            )
            .await
            {
                Ok(filter) => {
                    tracing::info!(
                        regex_patterns = config.content_filter.regex_patterns.len(),
                        excluded_topics = config.content_filter.excluded_topics.len(),
                        "Content filter enabled"
                    );
                    Some(Arc::new(filter))
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to initialize content filter -- filtering disabled");
                    None
                }
            }
        } else {
            None
        };

    // 7.5. Create summarization provider if enabled
    let summarization_provider = match create_summarization_provider(&config.summarization) {
        Ok(Some(provider)) => {
            let model = if config.summarization.provider == "openai" {
                &config.summarization.openai_model
            } else {
                &config.summarization.ollama_model
            };
            tracing::info!(
                provider = %config.summarization.provider,
                model = %model,
                "Summarization enabled for auto-store"
            );
            Some(provider)
        }
        Ok(None) => {
            tracing::info!("Summarization disabled — auto-store will store raw content");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to initialize summarization provider — summarization disabled");
            None
        }
    };

    // 8. Spawn auto-store sidecar if enabled
    if config.auto_store.enabled {
        if config.auto_store.filter_mode == "none" {
            tracing::warn!(
                "Auto-store filter_mode is \"none\" — every conversation turn will be stored. \
                 Set filter_mode = \"llm\" in [auto_store] config to filter for relevance (requires Ollama)."
            );
        }
        if config.auto_store.watch_paths.is_empty() {
            tracing::warn!("Auto-store enabled but watch_paths is empty — nothing will be ingested");
        }
        let _auto_store_handle = crate::auto_store::AutoStoreWorker::spawn(
            config.auto_store.clone(),
            store.clone(),
            Some(&pipeline),
            extraction_pipeline.as_ref(),
            &config.extraction,
            content_filter.clone(),
            summarization_provider,
        );
        tracing::info!("Auto-store sidecar spawned");
    }

    // 8.5. Spawn GC worker if enabled
    if config.gc.enabled {
        let gc_store = store.clone();
        let gc_config = config.gc.clone();
        let recall_config = config.recall.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(gc_config.gc_interval_secs));
            loop {
                interval.tick().await;
                match gc::run_gc(&gc_store, &gc_config, false).await {
                    Ok(result) => {
                        if result.pruned_count > 0 || result.expired_count > 0 || result.hard_purged_count > 0 {
                            tracing::info!(
                                pruned = result.pruned_count,
                                expired = result.expired_count,
                                hard_purged = result.hard_purged_count,
                                "GC cycle complete"
                            );
                        } else {
                            tracing::debug!("GC cycle complete — nothing to prune");
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "GC cycle failed"),
                }
                // IDP TTL cleanup: sweep expired idempotency keys on every GC cycle
                match gc_store.cleanup_expired_idempotency_keys().await {
                    Ok(0) => {}
                    Ok(n) => tracing::debug!(count = n, "Cleaned up expired idempotency keys"),
                    Err(e) => tracing::warn!(error = %e, "Failed to clean up expired idempotency keys"),
                }
                // Session recall auto-expiry: clean up sessions idle > session_idle_secs
                match gc_store.cleanup_expired_sessions(recall_config.session_idle_secs).await {
                    Ok(0) => {}
                    Ok(cleaned) => tracing::info!(cleaned, "Cleaned up expired recall sessions"),
                    Err(e) => tracing::warn!(error = %e, "Session cleanup failed"),
                }
            }
        });
        tracing::info!(
            interval_secs = config.gc.gc_interval_secs,
            "GC worker started"
        );
    } else {
        tracing::info!("GC disabled via config");
    }

    // 9. Write initial heartbeat
    write_heartbeat(&store).await;

    // Write embedding model info and watched file count (one-time on startup)
    {
        let model_name = &config.embedding.local_model;
        let model_dim = crate::embedding::model_dimension(model_name).unwrap_or(0) as i32;

        // Count watched JSONL files using existing watcher utilities
        let watched_count: i32 = if config.auto_store.enabled {
            config.auto_store.watch_paths.iter()
                .map(|p| {
                    let expanded = crate::auto_store::watcher::expand_tilde(p);
                    if expanded.is_dir() {
                        crate::auto_store::watcher::scan_directory_jsonl(&expanded).len() as i32
                    } else if expanded.is_file() {
                        1
                    } else {
                        0
                    }
                })
                .sum()
        } else {
            0
        };

        if let Err(e) = sqlx::query(
            "UPDATE daemon_status SET \
                 embedding_model = $1, embedding_dimension = $2, watched_file_count = $3 \
             WHERE id = 1"
        )
        .bind(model_name)
        .bind(model_dim)
        .bind(watched_count)
        .execute(store.pool())
        .await
        {
            tracing::warn!(error = %e, "Failed to write startup metadata to daemon_status");
        }

        tracing::info!(
            embedding_model = %model_name,
            embedding_dimension = model_dim,
            watched_file_count = watched_count,
            "Startup metadata written to daemon_status"
        );
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        "memcp daemon running"
    );

    // 10. Main loop: heartbeat every 30s, poll for pending work every 10s
    let poll_store = store.clone();
    let extraction_sender = extraction_pipeline.as_ref().map(|ep| ep.sender());
    let embedding_sender = pipeline.sender();

    let poll_handle = tokio::spawn(async move {
        let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(30));
        let mut poll_interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                _ = heartbeat_interval.tick() => {
                    write_heartbeat(&poll_store).await;
                }
                _ = poll_interval.tick() => {
                    // Poll for pending embeddings
                    match poll_store.get_pending_memories(100).await {
                        Ok(pending) if !pending.is_empty() => {
                            let count = pending.len();
                            for memory in pending {
                                let text = crate::embedding::build_embedding_text(
                                    &memory.content,
                                    &memory.tags,
                                );
                                let _ = embedding_sender.try_send(crate::embedding::EmbeddingJob {
                                    memory_id: memory.id,
                                    text,
                                    attempt: 0,
                                });
                            }
                            tracing::debug!(count = count, "Polled and queued pending embeddings");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to poll pending embeddings");
                        }
                        _ => {}
                    }

                    // Poll for pending extractions
                    if let Some(ref ext_sender) = extraction_sender {
                        match poll_store.get_pending_extraction(100).await {
                            Ok(pending) if !pending.is_empty() => {
                                let count = pending.len();
                                for (memory_id, content) in pending {
                                    let _ = ext_sender.try_send(ExtractionJob {
                                        memory_id,
                                        content,
                                        attempt: 0,
                                    });
                                }
                                tracing::debug!(count = count, "Polled and queued pending extractions");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to poll pending extractions");
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    });

    // 11. Wait for shutdown signal
    shutdown_signal().await;
    tracing::info!("Shutdown signal received, stopping daemon...");

    // 12. Clean shutdown
    poll_handle.abort();
    clear_heartbeat(&store).await;

    tracing::info!("memcp daemon stopped");
    Ok(())
}

/// Write a heartbeat to the daemon_status table.
async fn write_heartbeat(store: &PostgresMemoryStore) {
    let now = Utc::now();
    let pid = std::process::id() as i32;
    let version = env!("CARGO_PKG_VERSION");

    if let Err(e) = sqlx::query(
        "INSERT INTO daemon_status (id, last_heartbeat, started_at, pid, version) \
         VALUES (1, $1, $1, $2, $3) \
         ON CONFLICT (id) DO UPDATE SET last_heartbeat = $1, pid = $2, version = $3",
    )
    .bind(now)
    .bind(pid)
    .bind(version)
    .fetch_optional(store.pool())
    .await
    {
        tracing::warn!(error = %e, "Failed to write daemon heartbeat");
    }
}

/// Clear heartbeat on shutdown (set heartbeat and pid to NULL).
async fn clear_heartbeat(store: &PostgresMemoryStore) {
    if let Err(e) = sqlx::query(
        "UPDATE daemon_status SET last_heartbeat = NULL, pid = NULL WHERE id = 1",
    )
    .execute(store.pool())
    .await
    {
        tracing::warn!(error = %e, "Failed to clear daemon heartbeat");
    }
}

/// Wait for SIGINT (Ctrl+C) or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

/// Install the daemon as a system service (macOS launchd or Linux systemd).
pub fn install_service() -> Result<()> {
    let binary_path = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Failed to determine binary path: {}", e))?
        .to_string_lossy()
        .to_string();

    if cfg!(target_os = "macos") {
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("memcp")
            .join("logs");
        std::fs::create_dir_all(&log_dir)?;
        install_launchd(&binary_path, &log_dir.to_string_lossy())?;
    } else if cfg!(target_os = "linux") {
        install_systemd(&binary_path)?;
    } else {
        anyhow::bail!("Service installation not supported on this platform");
    }

    Ok(())
}

/// Install as a macOS launchd user agent.
fn install_launchd(binary_path: &str, log_dir: &str) -> Result<()> {
    let template = include_str!("../contrib/launchd/com.memcp.daemon.plist");
    let plist = template
        .replace("MEMCP_BINARY_PATH", binary_path)
        .replace("MEMCP_LOG_DIR", log_dir);

    let plist_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join("Library")
        .join("LaunchAgents");
    std::fs::create_dir_all(&plist_dir)?;

    let plist_path = plist_dir.join("com.memcp.daemon.plist");
    std::fs::write(&plist_path, plist)?;

    println!("Installed launchd plist at: {}", plist_path.display());
    println!("To load:   launchctl load {}", plist_path.display());
    println!("To unload: launchctl unload {}", plist_path.display());

    Ok(())
}

/// Install as a Linux systemd user service.
fn install_systemd(binary_path: &str) -> Result<()> {
    let template = include_str!("../contrib/systemd/memcp-daemon.service");
    let unit = template.replace("MEMCP_BINARY_PATH", binary_path);

    let systemd_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine config directory"))?
        .join("systemd")
        .join("user");
    std::fs::create_dir_all(&systemd_dir)?;

    let service_path = systemd_dir.join("memcp-daemon.service");
    std::fs::write(&service_path, unit)?;

    println!("Installed systemd unit at: {}", service_path.display());
    println!("To enable: systemctl --user enable memcp-daemon");
    println!("To start:  systemctl --user start memcp-daemon");

    Ok(())
}

/// Create the embedding provider based on configuration.
pub async fn create_embedding_provider(
    config: &Config,
) -> Result<Arc<dyn EmbeddingProvider + Send + Sync>> {
    match config.embedding.provider.as_str() {
        "openai" => {
            let api_key = config
                .embedding
                .openai_api_key
                .clone()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OpenAI API key required when provider is 'openai'. \
                         Set MEMCP_EMBEDDING__OPENAI_API_KEY or embedding.openai_api_key in memcp.toml"
                    )
                })?;
            let provider = OpenAIEmbeddingProvider::new(
                api_key,
                Some(config.embedding.openai_model.clone()),
                config.embedding.dimension,
            )?;
            // Warn if user-supplied dimension override differs from the model's known dimension
            if let Some(override_dim) = config.embedding.dimension {
                let detected = provider.dimension();
                if override_dim != detected {
                    tracing::warn!(
                        override_dim = override_dim,
                        detected_dim = detected,
                        model = %config.embedding.openai_model,
                        "embedding.dimension override differs from model's known dimension — using override"
                    );
                }
            }
            Ok(Arc::new(provider))
        }
        #[cfg(feature = "local-embed")]
        "local" | _ => Ok(Arc::new(
            LocalEmbeddingProvider::new(&config.embedding.cache_dir, &config.embedding.local_model).await?,
        )),
        #[cfg(not(feature = "local-embed"))]
        "local" => {
            anyhow::bail!(
                "Local embedding provider requires the 'local-embed' feature. \
                 Build with: cargo build --features local-embed\n\
                 Or switch to OpenAI: set embedding.provider = \"openai\" in memcp.toml"
            );
        }
        #[cfg(not(feature = "local-embed"))]
        _ => {
            anyhow::bail!(
                "Unknown embedding provider. When built without 'local-embed', \
                 only 'openai' is supported. Set embedding.provider = \"openai\" in memcp.toml"
            );
        }
    }
}

/// Create the extraction provider based on configuration.
pub fn create_extraction_provider(
    config: &Config,
) -> Result<Arc<dyn ExtractionProvider + Send + Sync>> {
    match config.extraction.provider.as_str() {
        "openai" => {
            let api_key = config
                .extraction
                .openai_api_key
                .clone()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OpenAI API key required when extraction provider is 'openai'. \
                         Set MEMCP_EXTRACTION__OPENAI_API_KEY or extraction.openai_api_key in memcp.toml"
                    )
                })?;
            Ok(Arc::new(OpenAIExtractionProvider::new(
                api_key,
                config.extraction.openai_model.clone(),
                config.extraction.max_content_chars,
            )?))
        }
        "ollama" | _ => Ok(Arc::new(OllamaExtractionProvider::new(
            config.extraction.ollama_base_url.clone(),
            config.extraction.ollama_model.clone(),
            config.extraction.max_content_chars,
        ))),
    }
}

/// Create the QI reranking provider based on configuration.
///
/// Used by the IPC listener to serve rerank requests from CLI processes, enabling
/// full pipeline parity between CLI search and MCP serve (SCF-01 gap closure).
pub fn create_qi_reranking_provider(
    config: &Config,
) -> Result<Arc<dyn QueryIntelligenceProvider + Send + Sync>> {
    match config.query_intelligence.reranking_provider.as_str() {
        "openai" => {
            let api_key = config
                .query_intelligence
                .openai_api_key
                .clone()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OpenAI API key required when query intelligence reranking provider is 'openai'. \
                         Set MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY or query_intelligence.openai_api_key in memcp.toml"
                    )
                })?;
            let provider = OpenAIQueryIntelligenceProvider::new(
                config.query_intelligence.openai_base_url.clone(),
                api_key,
                config.query_intelligence.reranking_openai_model.clone(),
            )
            .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(Arc::new(provider))
        }
        "ollama" | _ => Ok(Arc::new(OllamaQueryIntelligenceProvider::new(
            config.query_intelligence.ollama_base_url.clone(),
            config.query_intelligence.reranking_ollama_model.clone(),
        ))),
    }
}
