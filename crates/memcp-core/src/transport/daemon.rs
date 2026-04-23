//! Daemon mode -- long-running process hosting background workers.
//!
//! Runs embedding pipeline, extraction pipeline, consolidation worker,
//! auto-store sidecar, and content filter as background tasks.
//! Writes heartbeat to `daemon_status` table every 30 seconds.
//! Polls for pending embedding/extraction work every 10 seconds.

use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;

use crate::config::{Config, EmbeddingTierConfig};
use crate::consolidation::ConsolidationWorker;
use crate::content_filter::CompositeFilter;
use crate::curation::{self, create_curation_provider};
#[cfg(feature = "local-embed")]
use crate::embedding::local::LocalEmbeddingProvider;
use crate::embedding::openai::OpenAIEmbeddingProvider;
use crate::embedding::pipeline::{backfill, EmbeddingPipeline};
use crate::embedding::router::EmbeddingRouter;
use crate::embedding::EmbeddingProvider;
use crate::enrichment::create_enrichment_provider;
use crate::enrichment::worker::run_enrichment;
use crate::extraction::ollama::OllamaExtractionProvider;
use crate::extraction::openai::OpenAIExtractionProvider;
use crate::extraction::pipeline::ExtractionPipeline;
use crate::extraction::{ExtractionJob, ExtractionProvider};
use crate::gc::{self, DedupWorker};
use crate::ipc::{embed_socket_path, start_embed_listener};
use crate::pipeline::abstraction::{create_abstraction_provider, worker::run_abstraction_worker};
use crate::query_intelligence::ollama::OllamaQueryIntelligenceProvider;
use crate::query_intelligence::openai::OpenAIQueryIntelligenceProvider;
use crate::query_intelligence::QueryIntelligenceProvider;
use crate::store::postgres::PostgresMemoryStore;
use crate::summarization::create_summarization_provider;

/// Main daemon entry point.
///
/// Initializes all background workers, runs backfill, then enters
/// a heartbeat + poll loop until SIGINT/SIGTERM.
pub async fn run_daemon(config: &Config, skip_migrate: bool) -> Result<()> {
    let run_migrations = !skip_migrate;

    // 0. Create readiness flag for health probes (false until DB + migrations + HNSW ready)
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // 1. Initialize store with exponential backoff (containers: DB may not be ready)
    let store = {
        let mut delay = std::time::Duration::from_secs(1);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match PostgresMemoryStore::new_with_pool_config(
                &config.database_url,
                run_migrations,
                &config.search,
                config.resource_caps.max_db_connections,
            )
            .await
            {
                Ok(mut s) => {
                    tracing::info!(database_url = %crate::errors::redact_url(&config.database_url), "PostgreSQL store initialized");
                    // Apply type-specific FSRS stability config before wrapping in Arc
                    s.set_retention_config(config.retention.clone());
                    break Arc::new(s);
                }
                Err(e) if tokio::time::Instant::now() < deadline => {
                    tracing::warn!(
                        error = %e,
                        delay_secs = delay.as_secs(),
                        "DB not ready, retrying..."
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(16));
                }
                Err(e) => {
                    tracing::error!(error = %e, "DB unreachable after 30s — exiting");
                    std::process::exit(1);
                }
            }
        }
    };

    // 2. Build embedding router (single-tier or multi-tier based on config)
    let router = build_embedding_router(config).await?;
    let router = Arc::new(router);
    let provider_for_filter: Arc<dyn EmbeddingProvider + Send + Sync> =
        router.default_provider().clone();

    // 2.5. Ensure HNSW indexes exist for all configured tiers.
    // In single-tier mode, creates one index. In multi-tier mode, creates a
    // partial index per tier with the correct dimension cast.
    if router.is_multi_model() {
        for (tier, dim) in router.tier_dimensions() {
            store
                .ensure_hnsw_index_for_tier(tier, dim)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to ensure HNSW index for tier {}: {}", tier, e)
                })?;
            tracing::info!(tier, dimension = dim, "HNSW index ready for tier");
        }
    } else {
        let dim = router.dimension();
        store
            .ensure_hnsw_index(dim)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to ensure HNSW index: {}", e))?;
        tracing::info!(dimension = dim, "HNSW index ready");
    }

    // Mark as ready for health probes (DB connected + migrations applied + HNSW ready)
    ready.store(true, std::sync::atomic::Ordering::Release);
    tracing::info!("Daemon ready — accepting health probes");

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
    // The listener serves embed, embed_multi (multi-tier), and rerank requests from short-lived
    // CLI processes over a Unix domain socket, enabling full pipeline parity with MCP serve.
    {
        let embed_provider = provider_for_filter.clone();
        let socket_path = embed_socket_path();

        // Create QI reranking provider if reranking is enabled.
        let qi_provider: Option<Arc<dyn QueryIntelligenceProvider + Send + Sync>> = if config
            .query_intelligence
            .reranking_enabled
        {
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

        // Pass router+store for multi-tier embed_multi IPC when multi-model is active.
        // Single-model daemons pass None — CLI falls back to single embed.
        let multi_tier = if router.is_multi_model() {
            tracing::info!("Multi-tier embed_multi IPC enabled (CLI dual-query search)");
            Some((router.clone(), store.clone()))
        } else {
            None
        };

        tokio::spawn(async move {
            start_embed_listener(socket_path, embed_provider, qi_provider, multi_tier).await;
        });
    }

    // 3.8. Spawn abstraction worker BEFORE embedding pipeline so abstracts are
    // generated before embeddings pick up memories. This prevents the race condition
    // where a memory gets embedded against full content when an abstract is pending.
    if config.abstraction.enabled {
        match create_abstraction_provider(&config.abstraction) {
            Ok(Some(abstraction_provider)) => {
                let abstraction_store = store.clone();
                let abstraction_config = config.abstraction.clone();
                tokio::spawn(async move {
                    run_abstraction_worker(
                        abstraction_store,
                        abstraction_provider,
                        abstraction_config,
                    )
                    .await;
                });
                tracing::info!(
                    provider = %config.abstraction.provider,
                    min_content_length = config.abstraction.min_content_length,
                    generate_overview = config.abstraction.generate_overview,
                    "Abstraction worker started"
                );
            }
            Ok(None) => {
                tracing::info!("Abstraction disabled via config");
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize abstraction provider — abstraction disabled");
            }
        }
    } else {
        tracing::info!("Abstraction disabled via config");
    }

    // 4. Create embedding pipeline (uses router for multi-tier support)
    // Must be created before health server so embed_sender can be shared via AppState.
    let pipeline = EmbeddingPipeline::new(
        router.clone(),
        store.clone(),
        1000,
        consolidation_sender,
        dedup_sender,
    );

    // 2.6. Construct redaction engine if enabled (secrets_enabled or pii_enabled)
    let redaction_engine: Option<Arc<crate::pipeline::redaction::RedactionEngine>> = if config
        .redaction
        .secrets_enabled
        || config.redaction.pii_enabled
    {
        match crate::pipeline::redaction::RedactionEngine::from_config(&config.redaction) {
            Ok(engine) => {
                tracing::info!(
                    secrets_enabled = config.redaction.secrets_enabled,
                    pii_enabled = config.redaction.pii_enabled,
                    "Redaction engine initialized"
                );
                Some(Arc::new(engine))
            }
            Err(e) => {
                // Fail-closed: if secrets_enabled is true (default), redaction MUST work
                if config.redaction.secrets_enabled {
                    tracing::error!(error = %e, "Failed to initialize redaction engine — exiting (fail-closed)");
                    std::process::exit(1);
                }
                tracing::warn!(error = %e, "Failed to initialize redaction engine — redaction disabled");
                None
            }
        }
    } else {
        tracing::info!("Redaction disabled (secrets_enabled=false, pii_enabled=false)");
        None
    };

    // 2.65. Build content filter if enabled — needed by both auto-store worker AND
    // /v1/ingest handler (via AppState). Moved ahead of health-server spawn in Phase
    // 24.5-03 so the ingest handler inherits the same content filter as auto-store (D-10).
    let content_filter: Option<Arc<dyn crate::content_filter::ContentFilter>> = if config
        .content_filter
        .enabled
    {
        match CompositeFilter::from_config(&config.content_filter, Some(provider_for_filter.clone())).await
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

    // 2.66. Create summarization provider if enabled — used by auto-store AND
    // /v1/ingest (assistant-role summarization per D-10 / D-12).
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
                "Summarization enabled"
            );
            Some(provider)
        }
        Ok(None) => {
            tracing::info!("Summarization disabled");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to initialize summarization provider — summarization disabled");
            None
        }
    };

    // 2.7. Spawn health HTTP server if enabled
    // AppState carries config, embed_provider, and embed_sender for /v1/* API handlers.
    let health_handle = if config.health.enabled {
        // Phase 24.5 D-02 boot-safety gate (threat T-24.5-01):
        // Refuse to start the API if we bind non-loopback without an ingest api_key.
        // MUST run before any `axum::serve` / `TcpListener::bind` so a misconfigured
        // daemon cannot expose unauthenticated /v1/ingest on a routable interface.
        if let Err(msg) = crate::transport::boot_safety::check_ingest_auth_safety(
            &config.health.bind,
            config.ingest.api_key.as_deref(),
        ) {
            eprintln!("ERROR: {msg}");
            std::process::exit(1);
        }

        let addr: std::net::SocketAddr = format!("{}:{}", config.health.bind, config.health.port)
            .parse()
            .unwrap_or_else(|_| std::net::SocketAddr::from(([0, 0, 0, 0], 9090)));

        // Install Prometheus recorder and register metric descriptions.
        // MUST be called before spawning pool metrics poller and before /metrics endpoint is hit.
        let metrics_handle = crate::transport::metrics::install_prometheus_recorder();
        crate::transport::metrics::describe_metrics();

        // Spawn pool metrics poller (updates active/idle gauges every poll_interval).
        let poll_interval = Duration::from_secs(config.observability.pool_poll_interval_secs);
        crate::transport::metrics::spawn_pool_metrics_poller(
            Arc::new(store.pool().clone()),
            poll_interval,
        );

        let auth = crate::transport::api::auth::AuthState::from_optional(
            config.ingest.api_key.clone(),
        );

        // Phase 25 Plan 08: load reasoning env keys + derive tenancy. Pro when
        // any non-ollama MEMCP_REASONING__<P>_API_KEY is set; BYOK otherwise.
        // Ollama-only env does not flip to Pro (T-25-08-07).
        let reasoning_creds = crate::transport::health::ReasoningCreds::from_env();
        let reasoning_tenancy = reasoning_creds.tenancy();

        let state = crate::health::AppState {
            ready: ready.clone(),
            started_at: tokio::time::Instant::now(),
            caps: config.resource_caps.clone(),
            store: Some(store.clone()),
            config: std::sync::Arc::new(config.clone()),
            embed_provider: Some(router.default_provider().clone()),
            embed_sender: Some(pipeline.sender()),
            metrics_handle,
            redaction_engine: redaction_engine.clone(),
            auth,
            content_filter: content_filter.clone(),
            summarization_provider: summarization_provider.clone(),
            // extract_sender is only available after extraction pipeline is built
            // below; for now /v1/ingest runs without the extraction queue. The
            // auto-store worker still gets the sender via its own context, so
            // extraction parity for file-watched memories is unchanged.
            extract_sender: None,
            // Phase 24.75 Plan 04: shared topic-embedding cache for /v1/memory/span.
            // Fresh per daemon boot; bounded to 100 entries inside compute_memory_span.
            topic_embedding_cache: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            reasoning_creds,
            reasoning_tenancy,
        };
        Some(tokio::spawn(crate::health::serve(addr, state)))
    } else {
        tracing::info!("Health HTTP server disabled via config");
        None
    };

    // 5. Run startup embedding backfill
    let queued = backfill(&store, &pipeline.sender()).await;
    if queued > 0 {
        tracing::info!(
            count = queued,
            "Startup backfill queued memories for embedding"
        );
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
                            tracing::info!(
                                count = count,
                                "Startup backfill queued memories for extraction"
                            );
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

    // (content_filter + summarization_provider were constructed above, before the
    // health-server spawn, so AppState can carry them for /v1/ingest — D-10.)

    // 8. Spawn auto-store sidecar if enabled
    if config.auto_store.enabled {
        if config.auto_store.filter_mode == "none" {
            tracing::warn!(
                "Auto-store filter_mode is \"none\" — every conversation turn will be stored. \
                 Set filter_mode = \"llm\" in [auto_store] config to filter for relevance (requires Ollama)."
            );
        }
        if config.auto_store.watch_paths.is_empty() {
            tracing::warn!(
                "Auto-store enabled but watch_paths is empty — nothing will be ingested"
            );
        }
        // Resolve project: MEMCP_PROJECT env var overrides config default (MEMCP_WORKSPACE as fallback)
        let resolved_project = std::env::var("MEMCP_PROJECT")
            .ok()
            .or_else(|| std::env::var("MEMCP_WORKSPACE").ok())
            .or_else(|| config.project.default_project.clone());

        let _auto_store_handle =
            crate::auto_store::AutoStoreWorker::spawn(crate::auto_store::AutoStoreContext {
                config: config.auto_store.clone(),
                extraction_config: &config.extraction,
                store: store.clone(),
                embedding_pipeline: Some(&pipeline),
                extraction_pipeline: extraction_pipeline.as_ref(),
                embedding_router: Some(router.clone()),
                content_filter: content_filter.clone(),
                redaction_engine: redaction_engine.clone(),
                summarization_provider,
                project: resolved_project,
                birth_year: config.user.birth_year,
            });
        tracing::info!("Auto-store sidecar spawned");
    }

    // 8.5. Spawn GC worker if enabled
    if config.gc.enabled {
        let gc_store = store.clone();
        let gc_config = config.gc.clone();
        let recall_config = config.recall.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(gc_config.gc_interval_secs));
            loop {
                interval.tick().await;
                match gc::run_gc(&gc_store, &gc_config, false).await {
                    Ok(result) => {
                        if result.pruned_count > 0
                            || result.expired_count > 0
                            || result.hard_purged_count > 0
                        {
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
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to clean up expired idempotency keys")
                    }
                }
                // Session recall auto-expiry: clean up sessions idle > session_idle_secs
                match gc_store
                    .cleanup_expired_sessions(recall_config.session_idle_secs)
                    .await
                {
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

    // 8.6. Spawn curation worker if enabled
    if config.curation.enabled {
        let curation_store = store.clone();
        let curation_config = config.curation.clone();
        let curation_provider = match create_curation_provider(&curation_config) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create curation provider — curation disabled");
                None
            }
        };
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(curation_config.interval_secs));
            loop {
                interval.tick().await;
                let provider_ref = curation_provider.as_deref();
                match curation::worker::run_curation(
                    &curation_store,
                    &curation_config,
                    provider_ref,
                    false,
                )
                .await
                {
                    Ok(result) => {
                        if let Some(reason) = &result.skipped_reason {
                            tracing::debug!(reason = %reason, "Curation pass skipped");
                        } else {
                            tracing::info!(
                                run_id = %result.run_id,
                                merged = result.merged_count,
                                flagged = result.flagged_stale_count,
                                strengthened = result.strengthened_count,
                                skipped = result.skipped_count,
                                candidates = result.candidates_processed,
                                clusters = result.clusters_found,
                                "Curation pass complete"
                            );
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "Curation pass failed"),
                }
            }
        });
        tracing::info!(
            interval_secs = config.curation.interval_secs,
            provider = config
                .curation
                .llm_provider
                .as_deref()
                .unwrap_or("algorithmic"),
            "Curation worker started"
        );
    } else {
        tracing::info!("Curation disabled via config");
    }

    // 8.65. Spawn enrichment worker if enabled
    if config.enrichment.enabled {
        if let Some(enrichment_provider) = create_enrichment_provider(&config.query_intelligence) {
            let enrichment_store = store.clone();
            let enrichment_config = config.enrichment.clone();
            // Use a watch channel for shutdown signaling (matches run_enrichment signature)
            let (enrichment_shutdown_tx, enrichment_shutdown_rx) =
                tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                if let Err(e) = run_enrichment(
                    enrichment_store,
                    enrichment_provider,
                    enrichment_config,
                    enrichment_shutdown_rx,
                )
                .await
                {
                    tracing::error!(error = %e, "Enrichment worker failed");
                }
            });
            // Note: enrichment_shutdown_tx is intentionally held here; dropping it would
            // immediately signal shutdown. We keep it alive for the process lifetime.
            // In practice the daemon exits via process shutdown, not this channel.
            std::mem::forget(enrichment_shutdown_tx);
            tracing::info!(
                sweep_interval_secs = config.enrichment.sweep_interval_secs,
                batch_limit = config.enrichment.batch_limit,
                neighbor_depth = config.enrichment.neighbor_depth,
                "Enrichment worker started"
            );
        } else {
            tracing::info!("Enrichment enabled but no QI provider configured — skipping");
        }
    } else {
        tracing::debug!("Enrichment disabled via config (set enrichment.enabled = true to enable)");
    }

    // 8.7. Spawn promotion sweep worker if multi-model is configured
    if router.is_multi_model() {
        if let Some(quality_tier_config) = config.embedding.tiers.get("quality") {
            if let Some(ref promotion_config) = quality_tier_config.promotion {
                let sweep_store = store.clone();
                let quality_provider = router
                    .provider("quality")
                    .expect("quality tier configured but provider missing")
                    .clone();
                let promo_config = promotion_config.clone();

                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(
                        promo_config.sweep_interval_minutes * 60,
                    ));
                    loop {
                        interval.tick().await;
                        match crate::promotion::worker::run_promotion_sweep(
                            &sweep_store,
                            &quality_provider,
                            &promo_config,
                            "fast",
                            "quality",
                        )
                        .await
                        {
                            Ok(result) => {
                                if let Some(reason) = &result.skipped_reason {
                                    tracing::debug!(reason = %reason, "Promotion sweep skipped");
                                } else if result.promoted_count > 0 {
                                    tracing::info!(
                                        promoted = result.promoted_count,
                                        failed = result.failed_count,
                                        candidates = result.candidates_evaluated,
                                        "Promotion sweep complete"
                                    );
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "Promotion sweep failed"),
                        }
                    }
                });
                tracing::info!(
                    interval_minutes = promotion_config.sweep_interval_minutes,
                    batch_cap = promotion_config.batch_cap,
                    "Promotion sweep worker started"
                );
            } else {
                tracing::info!("Quality tier configured without promotion rules — sweep disabled");
            }
        } else {
            tracing::debug!("No quality tier configured — promotion sweep disabled");
        }
    }

    // 8.8. Spawn temporal LLM background worker if enabled
    if config.temporal.llm_enabled {
        let temporal_store = store.clone();
        let temporal_config = config.temporal.clone();
        let temporal_birth_year = config.user.birth_year;
        // Create a broadcast channel for shutdown signal (1-slot buffer sufficient)
        let (_shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
        tokio::spawn(async move {
            crate::pipeline::temporal::run_temporal_worker(
                temporal_store,
                &temporal_config,
                temporal_birth_year,
                shutdown_rx,
            )
            .await;
        });
        tracing::info!(
            provider = %config.temporal.provider,
            "Temporal LLM background worker started"
        );
    } else {
        tracing::debug!("Temporal LLM worker disabled (set temporal.llm_enabled = true to enable)");
    }

    // 9. Write initial heartbeat
    write_heartbeat(&store).await;

    // Write embedding model info and watched file count (one-time on startup)
    {
        let model_name = router.model_name();
        let model_dim = router.dimension() as i32;

        // Count watched JSONL files using existing watcher utilities
        let watched_count: i32 = if config.auto_store.enabled {
            config
                .auto_store
                .watch_paths
                .iter()
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
             WHERE id = 1",
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
                                    memory.abstract_text.as_deref(),
                                    &memory.tags,
                                );
                                let _ = embedding_sender.try_send(crate::embedding::EmbeddingJob {
                                    memory_id: memory.id,
                                    text,
                                    attempt: 0,
                                    completion_tx: None,
                                    tier: "fast".to_string(),
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
    tracing::info!("SIGTERM/SIGINT received — initiating graceful shutdown");

    // 12. Stop accepting new work
    ready.store(false, std::sync::atomic::Ordering::Release);
    tracing::info!("Marked as not-ready — health probes will return 503");

    // 13. Graceful shutdown with 10-second timeout
    let shutdown_store = store.clone();
    match tokio::time::timeout(std::time::Duration::from_secs(10), async {
        // Stop polling for new work
        tracing::info!("Stopping poll worker...");
        poll_handle.abort();

        // Flush embedding pipeline: drop sender so in-flight items drain
        // Items still pending in DB remain as embedding_status='pending' for next startup
        tracing::info!("Flushing embedding pipeline (pending items persist to DB)...");
        drop(pipeline);

        // Clear heartbeat
        tracing::info!("Clearing daemon heartbeat...");
        clear_heartbeat(&shutdown_store).await;

        // Close DB connection pool
        tracing::info!("Closing DB connection pool...");
        shutdown_store.pool().close().await;

        tracing::info!("Clean shutdown complete");
    })
    .await
    {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!("Shutdown timeout (10s) exceeded — forcing exit");
        }
    }

    // Abort health server if running
    if let Some(handle) = health_handle {
        handle.abort();
    }

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
    if let Err(e) =
        sqlx::query("UPDATE daemon_status SET last_heartbeat = NULL, pid = NULL WHERE id = 1")
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
    let template = include_str!("../../contrib/launchd/com.memcp.daemon.plist");
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
    let template = include_str!("../../contrib/systemd/memcp-daemon.service");
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
                config.embedding.openai_base_url.clone(),
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
        _ => Ok(Arc::new(
            LocalEmbeddingProvider::new(&config.embedding.cache_dir, &config.embedding.local_model)
                .await?,
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
        _ => Ok(Arc::new(OllamaExtractionProvider::new(
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
        _ => Ok(Arc::new(OllamaQueryIntelligenceProvider::new(
            config.query_intelligence.ollama_base_url.clone(),
            config.query_intelligence.reranking_ollama_model.clone(),
        ))),
    }
}

/// Build an EmbeddingRouter from the config.
///
/// If `embedding.tiers` is empty (legacy mode), wraps the single legacy provider
/// in a single-tier router for backward compatibility. If tiers are configured,
/// creates a provider for each tier and assembles the router.
async fn build_embedding_router(config: &Config) -> Result<EmbeddingRouter> {
    use std::collections::HashMap;

    if config.embedding.tiers.is_empty() {
        // Legacy single-model mode: wrap existing provider in a single-tier router
        let provider = create_embedding_provider(config).await?;
        let mut tiers = HashMap::new();
        tiers.insert("fast".to_string(), (provider, None));
        Ok(EmbeddingRouter::new(tiers, "fast".to_string()))
    } else {
        // Multi-model mode: create a provider for each configured tier
        let mut tiers = HashMap::new();
        for (name, tier_config) in &config.embedding.tiers {
            let provider = create_tier_provider(config, tier_config).await?;
            tiers.insert(name.clone(), (provider, tier_config.routing.clone()));
        }
        // Default tier is "fast" if it exists, else first alphabetical
        let default = if tiers.contains_key("fast") {
            "fast".to_string()
        } else {
            tiers
                .keys()
                .next()
                .expect("at least one embedding tier must be configured")
                .clone()
        };
        tracing::info!(
            tiers = ?tiers.keys().collect::<Vec<_>>(),
            default_tier = %default,
            "Multi-model embedding router configured"
        );
        Ok(EmbeddingRouter::new(tiers, default))
    }
}

/// Create an embedding provider for a specific tier configuration.
///
/// Falls back to top-level config values for API keys and model names
/// when the tier doesn't specify its own.
async fn create_tier_provider(
    config: &Config,
    tier: &EmbeddingTierConfig,
) -> Result<Arc<dyn EmbeddingProvider + Send + Sync>> {
    match tier.provider.as_str() {
        "openai" => {
            let api_key = tier
                .openai_api_key
                .clone()
                .or_else(|| config.embedding.openai_api_key.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OpenAI API key required for openai embedding tier. \
                         Set openai_api_key on the tier or top-level embedding config."
                    )
                })?;
            let model = tier
                .model
                .clone()
                .unwrap_or_else(|| config.embedding.openai_model.clone());
            let provider = OpenAIEmbeddingProvider::new(
                api_key,
                Some(model),
                tier.dimension,
                tier.base_url.clone(),
            )?;
            Ok(Arc::new(provider))
        }
        #[cfg(feature = "local-embed")]
        _ => {
            let model = tier
                .model
                .clone()
                .unwrap_or_else(|| config.embedding.local_model.clone());
            Ok(Arc::new(
                LocalEmbeddingProvider::new(&config.embedding.cache_dir, &model).await?,
            ))
        }
        #[cfg(not(feature = "local-embed"))]
        "local" => {
            anyhow::bail!(
                "Local embedding provider requires the 'local-embed' feature. \
                 Build with: cargo build --features local-embed"
            );
        }
        #[cfg(not(feature = "local-embed"))]
        _ => {
            anyhow::bail!(
                "Unknown embedding tier provider '{}'. When built without 'local-embed', \
                 only 'openai' is supported.",
                tier.provider
            );
        }
    }
}
