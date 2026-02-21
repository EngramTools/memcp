use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use std::time::Duration;
use memcp::cli;
use memcp::config::Config;
use memcp::consolidation::ConsolidationWorker;
use memcp::embedding::EmbeddingProvider;
use memcp::embedding::local::LocalEmbeddingProvider;
use memcp::embedding::openai::OpenAIEmbeddingProvider;
use memcp::embedding::pipeline::{EmbeddingPipeline, backfill};
use memcp::extraction::ExtractionJob;
use memcp::extraction::ExtractionProvider;
use memcp::extraction::ollama::OllamaExtractionProvider;
use memcp::extraction::openai::OpenAIExtractionProvider;
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
        memcp get <id>\n  \
        memcp delete <id>\n  \
        memcp reinforce <id> [--rating good|easy]\n  \
        memcp status                Show daemon status\n  \
        memcp daemon                Start background workers\n  \
        memcp daemon install        Install as system service\n  \
        memcp serve                 Start MCP server (stdio)\n  \
        memcp migrate               Run database migrations\n  \
        memcp embed backfill|stats  Embedding management\n\n\
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
    },
    /// Search memories by keyword + metadata matching with salience ranking
    Search {
        query: String,
        #[arg(long, default_value = "10")]
        limit: i64,
        #[arg(long)]
        created_after: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        audience: Option<String>,
        #[arg(long)]
        verbose: bool,
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
    /// Reinforce a memory to boost its salience in future searches
    Reinforce {
        id: String,
        #[arg(long, default_value = "good")]
        rating: String,
    },
    /// Show daemon status and pending work counts
    Status,
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
        /// New model name to switch to (e.g., "text-embedding-3-small")
        #[arg(long)]
        model: String,
        /// Show what would happen without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

/// Create the extraction provider based on configuration.
fn create_extraction_provider(config: &Config) -> Result<Arc<dyn ExtractionProvider + Send + Sync>> {
    match config.extraction.provider.as_str() {
        "openai" => {
            let api_key = config.extraction.openai_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!(
                    "OpenAI API key required when extraction provider is 'openai'. \
                     Set MEMCP_EXTRACTION__OPENAI_API_KEY or extraction.openai_api_key in memcp.toml"
                ))?;
            Ok(Arc::new(OpenAIExtractionProvider::new(
                api_key,
                config.extraction.openai_model.clone(),
                config.extraction.max_content_chars,
            )?))
        }
        "ollama" | _ => {
            Ok(Arc::new(OllamaExtractionProvider::new(
                config.extraction.ollama_base_url.clone(),
                config.extraction.ollama_model.clone(),
                config.extraction.max_content_chars,
            )))
        }
    }
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
    match config.query_intelligence.reranking_provider.as_str() {
        "openai" => {
            let api_key = config.query_intelligence.openai_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!(
                    "OpenAI API key required when query intelligence reranking provider is 'openai'. \
                     Set MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY or query_intelligence.openai_api_key in memcp.toml"
                ))?;
            let provider = OpenAIQueryIntelligenceProvider::new(
                config.query_intelligence.openai_base_url.clone(),
                api_key,
                config.query_intelligence.reranking_openai_model.clone(),
            ).map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(Arc::new(provider))
        }
        "ollama" | _ => {
            Ok(Arc::new(OllamaQueryIntelligenceProvider::new(
                config.query_intelligence.ollama_base_url.clone(),
                config.query_intelligence.reranking_ollama_model.clone(),
            )))
        }
    }
}

/// Create the embedding provider based on configuration.
async fn create_embedding_provider(config: &Config) -> Result<Arc<dyn EmbeddingProvider + Send + Sync>> {
    match config.embedding.provider.as_str() {
        "openai" => {
            let api_key = config.embedding.openai_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!(
                    "OpenAI API key required when provider is 'openai'. \
                     Set MEMCP_EMBEDDING__OPENAI_API_KEY or embedding.openai_api_key in memcp.toml"
                ))?;
            Ok(Arc::new(OpenAIEmbeddingProvider::new(api_key)?))
        }
        "local" | _ => {
            Ok(Arc::new(LocalEmbeddingProvider::new(&config.embedding.cache_dir).await?))
        }
    }
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
                    // No consolidation during manual backfill — consolidation is a live trigger only
                    let pipeline = EmbeddingPipeline::new(provider, store.clone(), 1000, None);
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
                EmbedAction::SwitchModel { model, dry_run } => {
                    let stats = store.embedding_stats().await?;

                    if dry_run {
                        println!("DRY RUN — Switch model to '{}'", model);
                        println!("Current embedding stats:");
                        println!("{}", serde_json::to_string_pretty(&stats)?);
                        println!("\nThis would:");
                        println!("  - Mark all current embeddings as stale (is_current = false)");
                        println!("  - Set embedding_status = 'pending' for all affected memories");
                        println!("  - New embeddings will use model '{}' on next backfill", model);
                        println!("\nRun without --dry-run to apply.");
                    } else {
                        println!("Switching to model '{}'...", model);
                        let stale_count = store.mark_all_embeddings_stale().await?;
                        println!("Marked {} embeddings as stale.", stale_count);
                        println!("Run 'memcp embed backfill' to generate new embeddings with the new model.");
                    }
                }
            }
            return Ok(());
        }

        Commands::Store { content, type_hint, source, tags, actor, actor_type, audience } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_store(&store, content, type_hint, source, tags, actor, actor_type, audience).await?;
        }

        Commands::Search { query, limit, created_after, created_before, tags, audience, verbose } => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_search(&store, &config, query, limit, created_after, created_before, tags, audience, verbose).await?;
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

        Commands::Status => {
            let store = cli::connect_store(&config, cli.skip_migrate).await?;
            cli::cmd_status(&store).await?;
        }

        Commands::Daemon { .. } => {
            eprintln!("Daemon mode not yet implemented. Use 'memcp serve' for MCP mode.");
            std::process::exit(1);
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

            let pipeline = EmbeddingPipeline::new(provider, store.clone(), 1000, consolidation_sender);

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
            let service = MemoryService::new(
                store as Arc<dyn memcp::store::MemoryStore + Send + Sync>,
                Some(pipeline),
                Some(provider_for_search),
                Some(pg_store_for_search),
                config.salience.clone(),
                extraction_pipeline,
                qi_expansion_provider,
                qi_reranking_provider,
                config.query_intelligence.clone(),
                content_filter,
            );

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
