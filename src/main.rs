use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::time::Duration;
use memcp::cli;
use memcp::config::Config;
use memcp::consolidation::ConsolidationWorker;
use memcp::embedding::EmbeddingProvider;
use memcp::embedding::model_dimension;
use memcp::embedding::pipeline::{EmbeddingPipeline, backfill};
use memcp::extraction::ExtractionJob;
use memcp::extraction::ExtractionProvider;
use memcp::extraction::pipeline::ExtractionPipeline;
use memcp::logging;
use memcp::query_intelligence::QueryIntelligenceProvider;
use memcp::query_intelligence::ollama::OllamaQueryIntelligenceProvider;
use memcp::query_intelligence::openai::OpenAIQueryIntelligenceProvider;
use memcp::content_filter::CompositeFilter;
use memcp::server::MemoryService;
use memcp::store::postgres::PostgresMemoryStore;
use rmcp::ServiceExt;

#[derive(Parser)]
#[command(
    name = "memcp",
    version,
    about = "Memory server for AI agents",
    long_about = "memcp - Memory server for AI agents\n\n\
        USAGE:\n  \
        memcp store <content> [--type-hint fact] [--source user] [--tags a,b]\n  \
        memcp search <query> [--limit 10] [--tags a,b]\n  \
        memcp list [--type-hint fact] [--source user] [--limit 20]\n  \
        memcp recent [--since 30m] [--source openclaw] [--actor vita]\n  \
        memcp get <id>\n  \
        memcp delete <id>\n  \
        memcp reinforce <id> [--rating good|easy]\n  \
        memcp status                Show daemon status\n  \
        memcp daemon                Start background workers\n  \
        memcp daemon install        Install as system service\n  \
        memcp serve                 Start MCP server (stdio)\n  \
        memcp migrate               Run database migrations\n  \
        memcp embed backfill|stats  Embedding management\n  \
        memcp embed switch-model --model BGEBaseENV15 --dry-run\n  \
        memcp embed switch-model --model BGEBaseENV15 --yes\n  \
        memcp statusline install    Install Claude Code status line\n  \
        memcp statusline remove     Remove Claude Code status line\n  \
        memcp gc [--dry-run]        Run garbage collection\n\n\
        OUTPUT: JSON to stdout. Errors to stderr with non-zero exit code.",
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Skip automatic database migration on startup
    #[arg(long, global = true)]
    skip_migrate: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a new memory (writes to DB, exits immediately)
    Store {
        content: String,
        #[arg(long, default_value = "fact")]
        type_hint: String,
        #[arg(long, default_value = "default")]
        source: String,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long, default_value = "agent")]
        actor_type: String,
        #[arg(long, default_value = "global")]
        audience: String,
        /// Optional idempotency key for at-most-once store semantics.
        /// Repeated calls with the same key return the original memory.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Search memories by keyword + metadata matching with salience ranking
    Search {
        query: String,
        #[arg(long, default_value = "20")]
        limit: i64,
        #[arg(long)]
        created_after: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long, value_delimiter = ',')]
        source: Option<Vec<String>>,
        #[arg(long)]
        audience: Option<String>,
        /// Filter by memory type (e.g., fact, preference, instruction, decision)
        #[arg(long)]
        type_hint: Option<String>,
        #[arg(long)]
        verbose: bool,
        /// Output raw JSON matching MCP serve envelope (id always present)
        #[arg(long)]
        json: bool,
        /// Output one line per result: id_short score snippet [tags]
        #[arg(long)]
        compact: bool,
        /// Pagination cursor from a previous search (next_cursor value)
        #[arg(long)]
        cursor: Option<String>,
        /// Field projection (comma-separated: content,tags,id). Returns only specified fields.
        #[arg(long)]
        fields: Option<String>,
        /// Minimum salience threshold (0.0-1.0). Excludes results below this score.
        #[arg(long)]
        min_salience: Option<f64>,
    },
    /// List memories with optional filters and pagination
    List {
        #[arg(long)]
        type_hint: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        created_after: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long)]
        updated_after: Option<String>,
        #[arg(long)]
        updated_before: Option<String>,
        #[arg(long, default_value = "20")]
        limit: i64,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        actor: Option<String>,
        #[arg(long)]
        audience: Option<String>,
        #[arg(long)]
        verbose: bool,
    },
    /// Retrieve a memory by ID
    Get { id: String },
    /// Delete a memory by ID (permanent)
    Delete { id: String },
    /// Show recent memories (for session handoff — configurable time window)
    Recent {
        /// Time window, e.g. "30m", "1h", "2h", "1d" (default: "30m")
        #[arg(long, default_value = "30m")]
        since: String,
        /// Filter by source system (e.g. "openclaw", "claude-code")
        #[arg(long)]
        source: Option<String>,
        /// Filter by actor/agent name (e.g. "vita", "main")
        #[arg(long)]
        actor: Option<String>,
        /// Max results to return
        #[arg(long, default_value = "10")]
        limit: i64,
        #[arg(long)]
        verbose: bool,
    },
    /// Reinforce a memory to boost its salience in future searches
    Reinforce {
        id: String,
        #[arg(long, default_value = "good")]
        rating: String,
    },
    /// Show daemon status and pending work counts
    Status {
        /// Human-readable one-liner output
        #[arg(long)]
        pretty: bool,
        /// Deep health check (pings DB, Ollama, checks model cache, watch paths)
        #[arg(long)]
        check: bool,
    },
    /// Start background workers (embedding, extraction, consolidation, auto-store)
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },
    /// Start MCP server on stdio (backwards-compatible mode)
    Serve,
    /// Run database migrations and exit
    Migrate,
    /// Embedding management operations
    Embed {
        #[command(subcommand)]
        action: EmbedAction,
    },
    /// Manage Claude Code status line integration
    Statusline {
        #[command(subcommand)]
        action: StatuslineAction,
    },
    /// Run or preview garbage collection (prune low-salience and expired memories)
    Gc {
        /// Show candidates without making changes
        #[arg(long)]
        dry_run: bool,
        /// Override salience threshold (default: from config)
        #[arg(long)]
        salience_threshold: Option<f64>,
        /// Override minimum age in days (default: from config)
        #[arg(long)]
        min_age_days: Option<u32>,
    },
    /// Provide relevance feedback for a memory (useful or irrelevant)
    Feedback {
        /// Memory ID to provide feedback for
        id: String,
        /// Feedback signal: "useful" or "irrelevant"
        signal: String,
    },
    /// Recall relevant memories for automatic context injection
    Recall {
        /// Query text to find relevant memories
        query: String,
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
        /// Clear session recall history before recalling (for context compaction)
        #[arg(long)]
        reset: bool,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Install daemon as a system service (launchd on macOS, systemd on Linux)
    Install,
}

#[derive(Subcommand)]
enum EmbedAction {
    /// Queue all un-embedded or failed memories for re-embedding
    Backfill,
    /// Show embedding statistics (counts by model, pending, failed)
    Stats,
    /// Switch to a new embedding model (marks current embeddings as stale)
    SwitchModel {
        /// New model name to switch to (e.g., "text-embedding-3-small", "BGEBaseENV15")
        #[arg(long)]
        model: String,
        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
        /// Skip confirmation prompt for destructive cross-dimension switches (for scripted use)
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum StatuslineAction {
    /// Install status line script to ~/.claude/scripts/
    Install,
    /// Remove status line script
    Remove,
}

/// Create the extraction provider based on configuration.
fn create_extraction_provider(config: &Config) -> Result<Arc<dyn ExtractionProvider + Send + Sync>> {
    memcp::daemon::create_extraction_provider(config)
}

/// Create the QI expansion provider based on configuration.
fn create_qi_expansion_provider(config: &Config) -> Result<Arc<dyn QueryIntelligenceProvider + Send + Sync>> {
    match config.query_intelligence.expansion_provider.as_str() {
        "openai" => {
            let api_key = config.query_intelligence.openai_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!(
                    "OpenAI API key required when query intelligence expansion provider is 'openai'. \
                     Set MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY or query_intelligence.openai_api_key in memcp.toml"
                ))?;
            let provider = OpenAIQueryIntelligenceProvider::new(
                config.query_intelligence.openai_base_url.clone(),
                api_key,
                config.query_intelligence.expansion_openai_model.clone(),
            ).map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(Arc::new(provider))
        }
        "ollama" | _ => {
            Ok(Arc::new(OllamaQueryIntelligenceProvider::new(
                config.query_intelligence.ollama_base_url.clone(),
                config.query_intelligence.expansion_ollama_model.clone(),
            )))
        }
    }
}

/// Create the QI reranking provider based on configuration.
fn create_qi_reranking_provider(config: &Config) -> Result<Arc<dyn QueryIntelligenceProvider + Send + Sync>> {
    memcp::daemon::create_qi_reranking_provider(config)
}

/// Create the embedding provider based on configuration.
async fn create_embedding_provider(config: &Config) -> Result<Arc<dyn EmbeddingProvider + Send + Sync>> {
    memcp::daemon::create_embedding_provider(config).await
}

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI args
    let cli = Cli::parse();

    // 2. Load configuration
    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("Config error (using defaults): {}", e);
        Config::default()
    });

    // 3. Initialize logging FIRST (before any other output)
    // CRITICAL: logging goes to stderr only — stdout is reserved for JSON-RPC
    logging::init_logging(&config);

    // 4. Handle subcommands
    match cli.command {
        Commands::Migrate => {
            tracing::info!("Running database migrations...");
            // run_migrations=true, just connect and migrate then exit
            let _store = PostgresMemoryStore::new(&config.database_url, true)
                .await
                .expect("Failed to connect and run migrations");
            println!("Migrations completed successfully.");
            return Ok(());
        }

        Commands::Embed { action } => {
            let store = Arc::new(
                PostgresMemoryStore::new(&config.database_url, true)
                    .await
                    .expect("Failed to connect to database"),
            );

            match action {
                EmbedAction::Backfill => {
                    println!("Starting embedding backfill...");
                    let provider = create_embedding_provider(&config).await?;
                    // No consolidation or dedup during manual backfill — these are live triggers only
                    let pipeline = EmbeddingPipeline::new(provider, store.clone(), 1000, None, None);
                    let count = backfill(&store, &pipeline.sender()).await;
                    println!("Queued {} memories for embedding.", count);
                    // Wait briefly for some embeddings to process
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let stats = store.embedding_stats().await?;
                    println!("Current stats: {}", serde_json::to_string_pretty(&stats)?);
                }
                EmbedAction::Stats => {
                    let stats = store.embedding_stats().await?;
                    println!("{}", serde_json::to_string_pretty(&stats)?);
                }
                EmbedAction::SwitchModel { model, dry_run, yes } => {
                    // Resolve target model dimension from the known model registry
                    let target_dim = match model_dimension(&model) {
                        Some(d) => d,
                        None => {
                            eprintln!("Error: Unknown model '{}'.", model);
                            eprintln!("Supported models:");
                            eprintln!("  Local (fastembed):");
                            eprintln!("    AllMiniLML6V2      (384 dims)");
                            eprintln!("    BGESmallENV15      (384 dims)");
                            eprintln!("    AllMiniLML12V2     (384 dims)");
                            eprintln!("    BGEBaseENV15       (768 dims)");
                            eprintln!("    BGELargeENV15      (1024 dims)");
                            eprintln!("  OpenAI:");
                            eprintln!("    text-embedding-3-small  (1536 dims)");
                            eprintln!("    text-embedding-3-large  (3072 dims)");
                            eprintln!("    text-embedding-ada-002  (1536 dims)");
                            std::process::exit(1);
                        }
                    };

                    // Query current dimension from DB
                    let current_dim = store.current_embedding_dimension().await?;

                    // Count total memories (for dry-run and output)
                    let total_memories = sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM memories"
                    )
                    .fetch_one(store.pool())
                    .await
                    .unwrap_or(0) as u64;

                    match current_dim {
                        None => {
                            // Fresh DB — no embeddings exist yet
                            println!("No existing embeddings found.");
                            println!("Target model: {} ({} dims)", model, target_dim);
                            println!();
                            println!("No purge needed. Update your memcp.toml to set the new model:");
                            println!("  [embedding]");
                            println!("  local_model = \"{}\"   # or openai_model for OpenAI", model);
                            println!();
                            println!("Then restart the daemon to begin embedding {} memories.", total_memories);
                        }
                        Some(current) if current == target_dim => {
                            // Same dimension — safe swap, just mark stale
                            if dry_run {
                                println!("DRY RUN — Same-dimension model switch");
                                println!("  Current dimension: {} dims", current);
                                println!("  Target model:      {} ({} dims)", model, target_dim);
                                println!("  Operation:         mark-stale (safe — no purge needed)");
                                println!();
                                println!("Would mark all current embeddings as stale (is_current = false).");
                                println!("Would reset embedding_status = 'pending' for affected memories.");
                                println!("{} memories would need re-embedding.", total_memories);
                                println!();
                                println!("Run without --dry-run to apply.");
                            } else {
                                println!("Switching to model '{}' (same dimension: {} dims)...", model, target_dim);
                                let stale_count = store.mark_all_embeddings_stale().await?;
                                println!("Marked {} embeddings as stale.", stale_count);
                                println!();
                                println!("Next steps:");
                                println!("  1. Update memcp.toml: set embedding.local_model = \"{}\"", model);
                                println!("     (or embedding.openai_model for OpenAI models)");
                                println!("  2. Run 'memcp embed backfill' to re-embed with the new model.");
                            }
                        }
                        Some(current) => {
                            // Cross-dimension switch — destructive, requires --yes
                            if dry_run {
                                println!("DRY RUN — Cross-dimension model switch (DESTRUCTIVE)");
                                println!("  Current dimension: {} dims", current);
                                println!("  Target model:      {} ({} dims)", model, target_dim);
                                println!("  Operation:         PURGE all embeddings + drop HNSW index");
                                println!();
                                println!("WARNING: Dimensions differ ({} -> {}).", current, target_dim);
                                println!("Existing embeddings are incompatible with the new model.");
                                println!();
                                println!("Would perform:");
                                println!("  - Drop HNSW index (idx_memory_embeddings_hnsw)");
                                println!("  - Delete ALL embedding rows from memory_embeddings");
                                println!("  - Reset embedding_status = 'pending' for all {} memories", total_memories);
                                println!();
                                println!("Source memories are NOT deleted — only embedding vectors are removed.");
                                println!("After switch + daemon restart, all {} memories will be re-embedded.", total_memories);
                                println!();
                                println!("Run with --yes (and without --dry-run) to apply.");
                            } else if !yes {
                                eprintln!("WARNING: Switching from {} dims to {} dims.", current, target_dim);
                                eprintln!("This will purge ALL existing embeddings ({} rows).", total_memories);
                                eprintln!("Source memories are not deleted — only embedding vectors are removed.");
                                eprintln!();
                                eprintln!("Re-run with --yes to confirm:");
                                eprintln!("  memcp embed switch-model --model {} --yes", model);
                                std::process::exit(1);
                            } else {
                                // --yes confirmed — execute destructive switch
                                println!("Cross-dimension switch: {} dims -> {} dims", current, target_dim);
                                println!("Dropping HNSW index...");
                                store.drop_hnsw_index().await?;
                                println!("Purging all embeddings...");
                                let purged = store.purge_all_embeddings().await?;
                                println!("Purged {} embedding rows. {} memories reset to pending.", purged, total_memories);
                                println!();
                                println!("Next steps:");
                                println!("  1. Update memcp.toml: set embedding.local_model = \"{}\"", model);
                                println!("     (or embedding.openai_model for OpenAI models)");
                                println!("  2. Restart the daemon — it will recreate the HNSW index ({} dims)", target_dim);
                                println!("     and begin re-embedding all {} memories.", total_memories);
                            }
                        }
                    }
                }
            }
            return Ok(());
        }

        Commands::Store { content, type_hint, source, tags, actor, actor_type, audience, idempotency_key } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_store(&store, &config, content, type_hint, source, tags, actor, actor_type, audience, idempotency_key).await?;
        }

        Commands::Search { query, limit, created_after, created_before, tags, source, audience, type_hint, verbose, json, compact, cursor, fields, min_salience } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_search(&store, &config, query, limit, created_after, created_before, tags, source, audience, type_hint, verbose, json, compact, cursor, fields, min_salience).await?;
        }

        Commands::Recent { since, source, actor, limit, verbose } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_recent(&store, since, source, actor, limit, verbose).await?;
        }

        Commands::List { type_hint, source, created_after, created_before, updated_after, updated_before, limit, cursor, actor, audience, verbose } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_list(&store, type_hint, source, created_after, created_before, updated_after, updated_before, limit, cursor, actor, audience, verbose).await?;
        }

        Commands::Get { id } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_get(&store, &id).await?;
        }

        Commands::Delete { id } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_delete(&store, &id).await?;
        }

        Commands::Reinforce { id, rating } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_reinforce(&store, &id, &rating).await?;
        }

        Commands::Status { pretty, check } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_status(&store, &config, pretty, check).await?;
        }

        Commands::Daemon { action } => {
            match action {
                Some(DaemonAction::Install) => {
                    memcp::daemon::install_service()?;
                }
                None => {
                    memcp::daemon::run_daemon(&config, cli.skip_migrate).await?;
                }
            }
        }

        Commands::Statusline { action } => {
            match action {
                StatuslineAction::Install => cli::cmd_statusline_install()?,
                StatuslineAction::Remove => cli::cmd_statusline_remove()?,
            }
        }

        Commands::Gc { dry_run, salience_threshold, min_age_days } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_gc(&store, &config, dry_run, salience_threshold, min_age_days).await?;
        }

        Commands::Feedback { id, signal } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            if let Err(e) = cli::cmd_feedback(&store, &id, &signal).await {
                println!("{}", serde_json::json!({"ok": false, "error": e.to_string()}));
                std::process::exit(1);
            }
        }

        Commands::Recall { query, session_id, reset } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_recall(&store, &config, &query, session_id, reset).await?;
        }

        Commands::Serve => {
            // Start the MCP server
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "memcp server starting"
            );

            // 5. Initialize PostgreSQL store
            let run_migrations = !cli.skip_migrate;
            let store = Arc::new(
                PostgresMemoryStore::new(&config.database_url, run_migrations)
                    .await
                    .expect("Failed to initialize database"),
            );

            tracing::info!(database_url = %config.database_url, "PostgreSQL store initialized");

            // 6. Create embedding provider and pipeline
            let provider = create_embedding_provider(&config).await
                .expect("Failed to initialize embedding provider");
            let provider_for_search = provider.clone();  // Clone for MemoryService search

            // 6b. Create consolidation worker if enabled (must happen before embedding pipeline)
            // Consolidation is triggered indirectly via the embedding pipeline's completion callback.
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
                tracing::info!("Consolidation disabled via config (consolidation.enabled=false)");
                None
            };

            // Serve mode: no dedup sender (serve path is lightweight, dedup handled by daemon)
            let pipeline = EmbeddingPipeline::new(provider, store.clone(), 1000, consolidation_sender, None);

            // 7. Run startup backfill — queue any un-embedded memories from previous runs
            let queued = backfill(&store, &pipeline.sender()).await;
            if queued > 0 {
                tracing::info!(count = queued, "Startup backfill queued memories for embedding");
            }

            // 8. Create extraction pipeline if enabled
            let extraction_pipeline = if config.extraction.enabled {
                match create_extraction_provider(&config) {
                    Ok(extraction_provider) => {
                        let ep = ExtractionPipeline::new(extraction_provider, store.clone(), 1000);
                        // Queue pending extractions on startup (backfill)
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
                        tracing::warn!(error = %e, "Failed to initialize extraction provider — extraction disabled");
                        None
                    }
                }
            } else {
                tracing::info!("Extraction disabled via config (extraction.enabled=false)");
                None
            };

            // 8b. Build content filter if enabled
            let content_filter: Option<Arc<dyn memcp::content_filter::ContentFilter>> = if config.content_filter.enabled {
                match CompositeFilter::from_config(
                    &config.content_filter,
                    Some(provider_for_search.clone()),
                ).await {
                    Ok(filter) => {
                        tracing::info!(
                            regex_patterns = config.content_filter.regex_patterns.len(),
                            excluded_topics = config.content_filter.excluded_topics.len(),
                            "Content filter enabled"
                        );
                        Some(Arc::new(filter))
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to initialize content filter — filtering disabled");
                        None
                    }
                }
            } else {
                None
            };

            // 8c. Spawn auto-store sidecar if enabled
            if config.auto_store.enabled {
                let _auto_store_handle = memcp::auto_store::AutoStoreWorker::spawn(
                    config.auto_store.clone(),
                    store.clone(),
                    Some(&pipeline),
                    extraction_pipeline.as_ref(),
                    &config.extraction,
                    content_filter.clone(),
                    None, // Summarization only in daemon mode
                );
                tracing::info!("Auto-store sidecar spawned");
            }

            // 9. Create QI providers if enabled
            let qi_expansion_provider = if config.query_intelligence.expansion_enabled {
                match create_qi_expansion_provider(&config) {
                    Ok(p) => {
                        tracing::info!(provider = %config.query_intelligence.expansion_provider, "Query expansion enabled");
                        Some(p)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to init expansion provider — expansion disabled");
                        None
                    }
                }
            } else {
                None
            };

            let qi_reranking_provider = if config.query_intelligence.reranking_enabled {
                match create_qi_reranking_provider(&config) {
                    Ok(p) => {
                        tracing::info!(provider = %config.query_intelligence.reranking_provider, "Query reranking enabled");
                        Some(p)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to init reranking provider — reranking disabled");
                        None
                    }
                }
            } else {
                None
            };

            // 10. Create service with store, pipeline, embedding provider, salience config, extraction pipeline, and QI providers
            let pg_store_for_search = store.clone();
            let mut service = MemoryService::new(
                store as Arc<dyn memcp::store::MemoryStore + Send + Sync>,
                Some(pipeline),
                Some(provider_for_search),
                Some(pg_store_for_search),
                config.salience.clone(),
                config.search.clone(),
                extraction_pipeline,
                qi_expansion_provider,
                qi_reranking_provider,
                config.query_intelligence.clone(),
                content_filter,
            );
            service.set_recall_config(config.recall.clone(), config.extraction.enabled);
            service.set_resource_caps(config.resource_caps.clone());

            // 11. Serve via stdio transport
            let (stdin, stdout) = rmcp::transport::io::stdio();
            let server = service.serve((stdin, stdout)).await?;

            tracing::info!("memcp server running — awaiting tool calls via stdio");

            // 12. Wait for shutdown (client disconnects or signal)
            server.waiting().await?;

            tracing::info!("memcp server stopped");
        }
    }

    Ok(())
}
