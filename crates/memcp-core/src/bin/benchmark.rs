/// Benchmark CLI binary for LongMemEval and LoCoMo evaluation.
///
/// Dispatches to the appropriate runner based on --benchmark flag:
///   --benchmark longmemeval (default): LongMemEval accuracy benchmark
///   --benchmark locomo: LoCoMo F1 benchmark with per-sample isolation
///
/// Both runners append results to benchmark_history.jsonl after completion.
/// Supports single config or "all" for comparison across vector-only / hybrid / hybrid+qi.
/// CI integration via --subset (LongMemEval) and --min-accuracy (exit code threshold).
///
/// Requires the `local-embed` feature (fastembed). Build with:
///   cargo build --features local-embed --bin benchmark

#[cfg(feature = "local-embed")]
use clap::Parser;
#[cfg(feature = "local-embed")]
use std::collections::HashMap;
#[cfg(feature = "local-embed")]
use std::path::PathBuf;
#[cfg(feature = "local-embed")]
use std::sync::Arc;

#[cfg(feature = "local-embed")]
use memcp::benchmark::dataset::load_dataset;
#[cfg(feature = "local-embed")]
use memcp::benchmark::locomo::dataset::load_locomo_dataset;
#[cfg(feature = "local-embed")]
use memcp::benchmark::locomo::runner::run_locomo_benchmark;
#[cfg(feature = "local-embed")]
use memcp::benchmark::locomo::{
    LoCoMoIngestionMode, LoCoMoQuestionResult, load_locomo_checkpoint,
};
#[cfg(feature = "local-embed")]
use memcp::benchmark::report::{self, BenchmarkReport, HistoryEntry, append_history};
#[cfg(feature = "local-embed")]
use memcp::benchmark::runner::{load_checkpoint, run_benchmark};
#[cfg(feature = "local-embed")]
use memcp::benchmark::default_configs;
#[cfg(feature = "local-embed")]
use memcp::embedding::local::LocalEmbeddingProvider;
#[cfg(feature = "local-embed")]
use memcp::embedding::pipeline::EmbeddingPipeline;
#[cfg(feature = "local-embed")]
use memcp::intelligence::query_intelligence::openai::OpenAIQueryIntelligenceProvider;
#[cfg(feature = "local-embed")]
use memcp::intelligence::query_intelligence::QueryIntelligenceProvider;
#[cfg(feature = "local-embed")]
use memcp::config::SearchConfig;
#[cfg(feature = "local-embed")]
use memcp::store::postgres::PostgresMemoryStore;

#[cfg(feature = "local-embed")]
#[derive(Parser)]
#[command(
    name = "memcp-benchmark",
    about = "LongMemEval and LoCoMo benchmark runner for memcp"
)]
struct Cli {
    /// Which benchmark to run: "longmemeval" or "locomo"
    #[arg(long, default_value = "longmemeval")]
    benchmark: String,

    /// Path to dataset JSON (defaults to benchmark-specific path if not set)
    #[arg(long)]
    dataset: Option<PathBuf>,

    /// Ingestion mode for LoCoMo: "per-turn" or "per-session" (ignored for longmemeval)
    #[arg(long, default_value = "per-turn")]
    ingestion_mode: String,

    /// Search configuration: "vector-only", "hybrid", "hybrid+qi", or "all" for comparison
    #[arg(long, default_value = "hybrid")]
    config: String,

    /// Run only first N questions/samples (for CI speed)
    #[arg(long)]
    subset: Option<usize>,

    /// Minimum overall accuracy/F1 to pass (CI threshold, e.g. 0.60 for 60%)
    #[arg(long)]
    min_accuracy: Option<f64>,

    /// Output directory for per-run result files
    #[arg(long, default_value = "data/results")]
    output_dir: PathBuf,

    /// Path to JSONL history file for appending benchmark scores
    #[arg(long, default_value = "data/benchmark_history.jsonl")]
    history_file: PathBuf,

    /// Resume from checkpoint if available
    #[arg(long)]
    resume: bool,

    /// OpenAI API key (can also be set via OPENAI_API_KEY env var)
    #[arg(long, env = "OPENAI_API_KEY")]
    openai_api_key: String,

    /// Database URL (can also be set via DATABASE_URL env var)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// QI provider API key (e.g., OpenRouter key). Required when running hybrid+qi config.
    #[arg(long, env = "QI_API_KEY")]
    qi_api_key: Option<String>,

    /// QI provider base URL (OpenAI-compatible endpoint)
    #[arg(long, default_value = "https://openrouter.ai/api/v1")]
    qi_base_url: String,

    /// QI model name for both expansion and reranking
    #[arg(long, default_value = "google/gemini-2.5-flash-lite")]
    qi_model: String,

    /// Keep the benchmark schema after run (default: drop for clean ephemeral isolation)
    #[arg(long)]
    keep_schema: bool,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    #[cfg(feature = "local-embed")]
    return run().await;

    #[cfg(not(feature = "local-embed"))]
    {
        eprintln!(
            "Error: The benchmark binary requires the 'local-embed' feature.\n\
             Build with: cargo build --features local-embed --bin benchmark"
        );
        std::process::exit(1);
    }
}

#[cfg(feature = "local-embed")]
async fn run() -> Result<(), anyhow::Error> {
    // 1. Parse CLI args
    let cli = Cli::parse();

    // 2. Initialize tracing (stdout, info level)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 3. Dispatch based on --benchmark
    match cli.benchmark.as_str() {
        "longmemeval" => run_longmemeval(&cli).await,
        "locomo" => run_locomo(&cli).await,
        other => anyhow::bail!(
            "Unknown benchmark '{}'. Valid options: longmemeval, locomo",
            other
        ),
    }
}

/// Run the LongMemEval benchmark (existing pipeline, unchanged behavior).
#[cfg(feature = "local-embed")]
async fn run_longmemeval(cli: &Cli) -> Result<(), anyhow::Error> {
    let dataset_path = cli.dataset.clone().unwrap_or_else(|| {
        PathBuf::from("data/longmemeval/longmemeval_s_cleaned.json")
    });

    // Load dataset
    tracing::info!(path = %dataset_path.display(), "Loading LongMemEval dataset");
    let mut questions = load_dataset(&dataset_path)?;
    tracing::info!(count = questions.len(), "Dataset loaded");

    // Apply subset if specified (sort by question_id for reproducibility, then truncate)
    if let Some(n) = cli.subset {
        questions.sort_by(|a, b| a.question_id.cmp(&b.question_id));
        questions.truncate(n);
        tracing::info!(subset = n, "Applied subset — using {} questions", questions.len());
    }

    // Print summary
    let mut category_counts: HashMap<String, usize> = HashMap::new();
    for q in &questions {
        *category_counts.entry(q.category().to_string()).or_insert(0) += 1;
    }
    println!("=== LongMemEval Benchmark ===");
    println!("Dataset: {}", dataset_path.display());
    println!("Questions: {}", questions.len());
    println!("Per-category counts:");
    for cat in &[
        "information_extraction",
        "multi_session",
        "temporal_reasoning",
        "knowledge_update",
        "abstention",
    ] {
        let count = category_counts.get(*cat).copied().unwrap_or(0);
        println!("  {:<25} {}", format!("{}:", cat), count);
    }
    println!();

    // Create output directory
    std::fs::create_dir_all(&cli.output_dir)?;

    // Initialize database (isolated in benchmark schema)
    tracing::info!(database_url = %cli.database_url, schema = "benchmark", "Connecting to database");
    tracing::info!(schema = "benchmark", "Using isolated benchmark schema");
    let store = Arc::new(
        PostgresMemoryStore::new_with_schema(
            &cli.database_url,
            true,
            &SearchConfig::default(),
            Some("benchmark"),
        )
        .await?,
    );
    tracing::info!("Database ready");

    // Initialize embedding provider and pipeline
    tracing::info!("Initializing local embedding provider");
    let embedding_provider: Arc<dyn memcp::embedding::EmbeddingProvider + Send + Sync> =
        Arc::new(LocalEmbeddingProvider::new(".fastembed_cache", "AllMiniLML6V2").await?);
    let pipeline = EmbeddingPipeline::new_single(
        embedding_provider.clone(),
        store.clone(),
        1000,
        None,
        None,
    );

    // Construct QI provider if API key provided
    let qi_provider: Option<Arc<dyn QueryIntelligenceProvider>> =
        if let Some(ref api_key) = cli.qi_api_key {
            tracing::info!(base_url = %cli.qi_base_url, model = %cli.qi_model, "Initializing QI provider");
            let provider = OpenAIQueryIntelligenceProvider::new(
                cli.qi_base_url.clone(),
                api_key.clone(),
                cli.qi_model.clone(),
            )
            .map_err(|e| anyhow::anyhow!("Failed to create QI provider: {}", e))?;
            Some(Arc::new(provider))
        } else {
            None
        };

    // Determine configs to run
    let all_configs = default_configs();
    let configs_to_run: Vec<_> = if cli.config == "all" {
        all_configs.iter().collect()
    } else {
        let found = all_configs
            .iter()
            .find(|c| c.name == cli.config)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown config '{}'. Valid options: vector-only, hybrid, hybrid+qi, all",
                    cli.config
                )
            })?;
        vec![found]
    };

    if qi_provider.is_none() && configs_to_run.iter().any(|c| c.qi_expansion || c.qi_reranking) {
        tracing::warn!(
            "hybrid+qi config selected but --qi-api-key not provided. \
             QI expansion/reranking will be skipped (results identical to hybrid)."
        );
    }

    // Run each config
    let mut reports: Vec<BenchmarkReport> = Vec::new();

    for config in &configs_to_run {
        println!("--- Running config: {} ---", config.name);

        let checkpoint_path = cli
            .output_dir
            .join(format!("longmemeval_{}_checkpoint.json", config.name));

        let resume_state = if cli.resume {
            match load_checkpoint(&checkpoint_path) {
                Ok(Some(state)) => {
                    tracing::info!(
                        config = %config.name,
                        completed = state.completed_question_ids.len(),
                        "Resuming from checkpoint"
                    );
                    Some(state)
                }
                Ok(None) => {
                    tracing::info!(config = %config.name, "No checkpoint found — starting fresh");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to load checkpoint — starting fresh");
                    None
                }
            }
        } else {
            None
        };

        let results = run_benchmark(
            &questions,
            config,
            store.clone(),
            &pipeline,
            embedding_provider.clone(),
            &cli.openai_api_key,
            &checkpoint_path,
            resume_state,
            qi_provider.clone(),
        )
        .await?;

        let report = report::generate_report(&config.name, &results);
        report::print_report(&report);
        println!();

        let report_path = cli
            .output_dir
            .join(format!("longmemeval_{}_report.json", config.name));
        report::save_report(&report, &report_path)?;
        tracing::info!(path = %report_path.display(), "Report saved");

        // Append to benchmark history
        let history_entry = longmemeval_history_entry(config.name.as_str(), &report);
        if let Some(parent) = cli.history_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        append_history(&history_entry, &cli.history_file)?;
        tracing::info!(
            path = %cli.history_file.display(),
            config = %config.name,
            score = history_entry.overall_score,
            "History entry appended"
        );

        reports.push(report);
    }

    if reports.len() > 1 {
        report::print_comparison(&reports);
        println!();
    }

    // CI threshold check
    if let Some(threshold) = cli.min_accuracy {
        let last_report = reports.last().expect("At least one report must exist");
        if last_report.overall_accuracy < threshold {
            eprintln!(
                "FAIL: overall accuracy {:.1}% < threshold {:.1}%",
                last_report.overall_accuracy * 100.0,
                threshold * 100.0
            );
            std::process::exit(1);
        } else {
            println!(
                "PASS: overall accuracy {:.1}% >= threshold {:.1}%",
                last_report.overall_accuracy * 100.0,
                threshold * 100.0
            );
        }
    }

    // Schema cleanup — drop benchmark schema unless --keep-schema is passed
    if !cli.keep_schema {
        store.drop_schema().await?;
        tracing::info!("Benchmark schema dropped");
    } else {
        tracing::info!("Benchmark schema retained (--keep-schema)");
    }

    Ok(())
}

/// Run the LoCoMo benchmark.
#[cfg(feature = "local-embed")]
async fn run_locomo(cli: &Cli) -> Result<(), anyhow::Error> {
    let dataset_path = cli
        .dataset
        .clone()
        .unwrap_or_else(|| PathBuf::from("data/locomo/locomo10.json"));

    // Parse ingestion mode
    let ingestion_mode = match cli.ingestion_mode.as_str() {
        "per-turn" => LoCoMoIngestionMode::PerTurn,
        "per-session" => LoCoMoIngestionMode::PerSession,
        other => anyhow::bail!(
            "Unknown ingestion-mode '{}'. Valid options: per-turn, per-session",
            other
        ),
    };

    // Load dataset
    tracing::info!(path = %dataset_path.display(), "Loading LoCoMo dataset");
    let mut samples = load_locomo_dataset(&dataset_path)?;
    tracing::info!(count = samples.len(), "Dataset loaded");

    // Apply subset if specified
    if let Some(n) = cli.subset {
        samples.truncate(n);
        tracing::info!(subset = n, "Applied subset — using {} samples", samples.len());
    }

    // Print summary
    let total_qa: usize = samples.iter().map(|s| s.qa.len()).sum();
    let mut cat_counts: HashMap<u8, usize> = HashMap::new();
    for s in &samples {
        for qa in &s.qa {
            *cat_counts.entry(qa.category_u8()).or_insert(0) += 1;
        }
    }
    println!("=== LoCoMo Benchmark ===");
    println!("Dataset: {}", dataset_path.display());
    println!(
        "Ingestion mode: {}",
        match ingestion_mode {
            LoCoMoIngestionMode::PerTurn => "per-turn",
            LoCoMoIngestionMode::PerSession => "per-session",
        }
    );
    println!("Samples: {}", samples.len());
    println!("Total QA pairs: {}", total_qa);
    println!("Per-category QA counts:");
    for (cat_id, label) in &[
        (1u8, "single_hop"),
        (2u8, "multi_hop"),
        (3u8, "temporal"),
        (4u8, "commonsense"),
        (5u8, "adversarial"),
    ] {
        let count = cat_counts.get(cat_id).copied().unwrap_or(0);
        println!("  {:<25} {}", format!("{}:", label), count);
    }
    println!();

    // Create output directory
    std::fs::create_dir_all(&cli.output_dir)?;

    // Initialize database (isolated in benchmark schema)
    tracing::info!(database_url = %cli.database_url, schema = "benchmark", "Connecting to database");
    tracing::info!(schema = "benchmark", "Using isolated benchmark schema");
    let store = Arc::new(
        PostgresMemoryStore::new_with_schema(
            &cli.database_url,
            true,
            &SearchConfig::default(),
            Some("benchmark"),
        )
        .await?,
    );
    tracing::info!("Database ready");

    // Initialize embedding provider and pipeline
    tracing::info!("Initializing local embedding provider");
    let embedding_provider: Arc<dyn memcp::embedding::EmbeddingProvider + Send + Sync> =
        Arc::new(LocalEmbeddingProvider::new(".fastembed_cache", "AllMiniLML6V2").await?);
    let pipeline = EmbeddingPipeline::new_single(
        embedding_provider.clone(),
        store.clone(),
        1000,
        None,
        None,
    );

    // Construct QI provider if API key provided
    let qi_provider: Option<Arc<dyn QueryIntelligenceProvider>> =
        if let Some(ref api_key) = cli.qi_api_key {
            tracing::info!(base_url = %cli.qi_base_url, model = %cli.qi_model, "Initializing QI provider");
            let provider = OpenAIQueryIntelligenceProvider::new(
                cli.qi_base_url.clone(),
                api_key.clone(),
                cli.qi_model.clone(),
            )
            .map_err(|e| anyhow::anyhow!("Failed to create QI provider: {}", e))?;
            Some(Arc::new(provider))
        } else {
            None
        };

    // Determine configs to run
    let all_configs = default_configs();
    let configs_to_run: Vec<_> = if cli.config == "all" {
        all_configs.iter().collect()
    } else {
        let found = all_configs
            .iter()
            .find(|c| c.name == cli.config)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown config '{}'. Valid options: vector-only, hybrid, hybrid+qi, all",
                    cli.config
                )
            })?;
        vec![found]
    };

    if qi_provider.is_none() && configs_to_run.iter().any(|c| c.qi_expansion || c.qi_reranking) {
        tracing::warn!(
            "hybrid+qi config selected but --qi-api-key not provided. \
             QI expansion/reranking will be skipped (results identical to hybrid)."
        );
    }

    // Run each config
    let mut all_run_results: Vec<(String, Vec<LoCoMoQuestionResult>)> = Vec::new();

    for config in &configs_to_run {
        println!("--- Running config: {} ---", config.name);

        let checkpoint_path = cli
            .output_dir
            .join(format!("locomo_{}_checkpoint.json", config.name));

        let resume_state = if cli.resume {
            match load_locomo_checkpoint(&checkpoint_path) {
                Ok(Some(state)) => {
                    tracing::info!(
                        config = %config.name,
                        completed = state.completed_sample_ids.len(),
                        "Resuming from checkpoint"
                    );
                    Some(state)
                }
                Ok(None) => {
                    tracing::info!(config = %config.name, "No checkpoint found — starting fresh");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to load checkpoint — starting fresh");
                    None
                }
            }
        } else {
            None
        };

        let results = run_locomo_benchmark(
            &samples,
            config,
            &ingestion_mode,
            store.clone(),
            &pipeline,
            embedding_provider.clone(),
            &cli.openai_api_key,
            &checkpoint_path,
            resume_state,
            qi_provider.clone(),
        )
        .await?;

        // Generate and print report
        let locomo_report = generate_locomo_report(&config.name, &ingestion_mode, &results);
        print_locomo_report(&locomo_report);
        println!();

        // Save JSON report
        let report_path = cli
            .output_dir
            .join(format!("locomo_{}_report.json", config.name));
        let json = serde_json::to_string_pretty(&locomo_report)?;
        std::fs::write(&report_path, json)?;
        tracing::info!(path = %report_path.display(), "LoCoMo report saved");

        // Append to benchmark history
        let history_entry = locomo_history_entry(config.name.as_str(), &locomo_report);
        if let Some(parent) = cli.history_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        append_history(&history_entry, &cli.history_file)?;
        tracing::info!(
            path = %cli.history_file.display(),
            config = %config.name,
            f1 = history_entry.overall_score,
            "History entry appended"
        );

        all_run_results.push((config.name.clone(), results));
    }

    // CI threshold check (against task-averaged F1 of last config)
    if let Some(threshold) = cli.min_accuracy {
        let (config_name, last_results) = all_run_results
            .last()
            .expect("At least one config must have run");
        let report = generate_locomo_report(config_name, &ingestion_mode, last_results);
        if report.task_averaged_f1 < threshold {
            eprintln!(
                "FAIL: task-averaged F1 {:.1}% < threshold {:.1}%",
                report.task_averaged_f1 * 100.0,
                threshold * 100.0
            );
            std::process::exit(1);
        } else {
            println!(
                "PASS: task-averaged F1 {:.1}% >= threshold {:.1}%",
                report.task_averaged_f1 * 100.0,
                threshold * 100.0
            );
        }
    }

    // Schema cleanup — drop benchmark schema unless --keep-schema is passed
    if !cli.keep_schema {
        store.drop_schema().await?;
        tracing::info!("Benchmark schema dropped");
    } else {
        tracing::info!("Benchmark schema retained (--keep-schema)");
    }

    Ok(())
}

// ─── LoCoMo Report Types ─────────────────────────────────────────────────────

/// Per-category F1 statistics for a LoCoMo report.
#[cfg(feature = "local-embed")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LoCoMoCategoryReport {
    pub mean_f1: f64,
    pub count: usize,
}

/// Full LoCoMo report for a single config run.
#[cfg(feature = "local-embed")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LoCoMoRunReport {
    pub config_name: String,
    pub ingestion_mode: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Mean of all per-question F1 scores.
    pub overall_f1: f64,
    /// Mean of per-category mean F1 scores (official LoCoMo metric).
    pub task_averaged_f1: f64,
    pub question_count: usize,
    pub per_category: HashMap<String, LoCoMoCategoryReport>,
    pub mean_latency_ms: u64,
    pub p95_latency_ms: u64,
}

/// Generate a LoCoMo run report from results.
#[cfg(feature = "local-embed")]
fn generate_locomo_report(
    config_name: &str,
    ingestion_mode: &LoCoMoIngestionMode,
    results: &[LoCoMoQuestionResult],
) -> LoCoMoRunReport {
    let ingestion_mode_str = match ingestion_mode {
        LoCoMoIngestionMode::PerTurn => "per-turn",
        LoCoMoIngestionMode::PerSession => "per-session",
    };

    // Group by category
    let mut cat_f1s: HashMap<u8, Vec<f64>> = HashMap::new();
    let mut latencies: Vec<u64> = Vec::with_capacity(results.len());

    for r in results {
        cat_f1s.entry(r.category).or_default().push(r.f1);
        latencies.push(r.latency_ms);
    }

    // Per-category mean F1
    let per_category: HashMap<String, LoCoMoCategoryReport> = cat_f1s
        .iter()
        .map(|(&cat_id, f1s)| {
            let mean_f1 = f1s.iter().sum::<f64>() / f1s.len() as f64;
            let label = memcp::benchmark::locomo::category_label(cat_id).to_string();
            (label, LoCoMoCategoryReport { mean_f1, count: f1s.len() })
        })
        .collect();

    // Overall F1: mean of all per-question F1 scores
    let overall_f1 = if results.is_empty() {
        0.0
    } else {
        results.iter().map(|r| r.f1).sum::<f64>() / results.len() as f64
    };

    // Task-averaged F1: mean of per-category mean F1 scores (official metric)
    let task_averaged_f1 = if per_category.is_empty() {
        0.0
    } else {
        per_category.values().map(|c| c.mean_f1).sum::<f64>() / per_category.len() as f64
    };

    // Latency stats
    let mean_latency_ms = if latencies.is_empty() {
        0
    } else {
        latencies.iter().sum::<u64>() / latencies.len() as u64
    };
    let p95_latency_ms = if latencies.is_empty() {
        0
    } else {
        let mut sorted = latencies.clone();
        sorted.sort_unstable();
        let idx = ((0.95 * sorted.len() as f64).ceil() as usize).saturating_sub(1);
        sorted[idx.min(sorted.len() - 1)]
    };

    LoCoMoRunReport {
        config_name: config_name.to_string(),
        ingestion_mode: ingestion_mode_str.to_string(),
        timestamp: chrono::Utc::now(),
        overall_f1,
        task_averaged_f1,
        question_count: results.len(),
        per_category,
        mean_latency_ms,
        p95_latency_ms,
    }
}

/// Print a formatted LoCoMo report to stdout.
#[cfg(feature = "local-embed")]
fn print_locomo_report(report: &LoCoMoRunReport) {
    println!(
        "=== LoCoMo Benchmark Report: {} ({}) ===",
        report.config_name, report.ingestion_mode
    );
    println!("Date: {}", report.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
    println!("Questions: {}", report.question_count);
    println!("Overall F1: {:.1}%", report.overall_f1 * 100.0);
    println!(
        "Task-Averaged F1: {:.1}%",
        report.task_averaged_f1 * 100.0
    );
    println!();
    println!("Per-Category F1:");

    let ordered = [
        "single_hop",
        "multi_hop",
        "temporal",
        "commonsense",
        "adversarial",
    ];
    for cat in &ordered {
        if let Some(c) = report.per_category.get(*cat) {
            println!(
                "  {:<25}  {} questions, F1={:.1}%",
                format!("{}:", cat),
                c.count,
                c.mean_f1 * 100.0
            );
        }
    }
    for (cat, c) in &report.per_category {
        if !ordered.contains(&cat.as_str()) {
            println!(
                "  {:<25}  {} questions, F1={:.1}%",
                format!("{}:", cat),
                c.count,
                c.mean_f1 * 100.0
            );
        }
    }

    println!();
    println!(
        "Latency: mean={}ms, p95={}ms",
        report.mean_latency_ms, report.p95_latency_ms
    );
}

// ─── History Entry Builders ───────────────────────────────────────────────────

/// Build a HistoryEntry from a LongMemEval BenchmarkReport.
#[cfg(feature = "local-embed")]
fn longmemeval_history_entry(config_name: &str, report: &BenchmarkReport) -> HistoryEntry {
    let per_category: HashMap<String, f64> = report
        .categories
        .iter()
        .map(|(k, v)| (k.clone(), v.accuracy))
        .collect();

    HistoryEntry {
        timestamp: chrono::Utc::now(),
        benchmark: "longmemeval".to_string(),
        config_name: config_name.to_string(),
        git_sha: std::env::var("GIT_SHA").ok(),
        overall_score: report.overall_accuracy,
        task_averaged_score: report.task_averaged_accuracy,
        question_count: report.total_questions,
        per_category,
    }
}

/// Build a HistoryEntry from a LoCoMo run report.
#[cfg(feature = "local-embed")]
fn locomo_history_entry(config_name: &str, report: &LoCoMoRunReport) -> HistoryEntry {
    let per_category: HashMap<String, f64> = report
        .per_category
        .iter()
        .map(|(k, v)| (k.clone(), v.mean_f1))
        .collect();

    HistoryEntry {
        timestamp: chrono::Utc::now(),
        benchmark: "locomo".to_string(),
        config_name: config_name.to_string(),
        git_sha: std::env::var("GIT_SHA").ok(),
        overall_score: report.overall_f1,
        task_averaged_score: report.task_averaged_f1,
        question_count: report.question_count,
        per_category,
    }
}
