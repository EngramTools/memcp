//! Load test binary for memcp capacity sizing.
//!
//! Orchestrates the full load test lifecycle:
//!   1. Connect to Postgres, run migrations
//!   2. For each corpus size: clear + seed corpus, spawn test server
//!   3. For each concurrency level: run workload (raw + rate-limited if applicable)
//!   4. Aggregate results into LoadTestReport, save JSON + Markdown, print summary
//!
//! Usage:
//!   DATABASE_URL=postgres://... cargo run --bin load-test -- --help

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use axum::Router;
use axum::routing::get;
use clap::Parser;
use chrono::Utc;
use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::time::Instant;

use memcp::MIGRATOR;
use memcp::config::{Config, RateLimitConfig};
use memcp::load_test::{LoadTestConfig, LoadTestReport, WorkloadProfile};
use memcp::load_test::{client, corpus, metrics as lt_metrics, report};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::transport::health::AppState;
use memcp::transport::api;

// ─── CLI Definition ───────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "memcp-load-test", about = "HTTP load test harness for memcp capacity sizing")]
struct Cli {
    /// Corpus size: number of memories to seed before testing.
    /// If not specified, tests run at 100, 1000, and 10000.
    #[arg(long)]
    corpus_size: Option<usize>,

    /// Concurrency level: number of simultaneous clients.
    /// If not specified, tests run at 10, 50, 100, and 500.
    #[arg(long)]
    concurrency: Option<usize>,

    /// Read/Write ratio: "80/20" (read-heavy), "50/50" (balanced), "20/80" (write-heavy)
    #[arg(long, default_value = "80/20")]
    rw_ratio: String,

    /// Test duration in seconds (used for display only — workload is op-count based)
    #[arg(long, default_value = "30")]
    duration: u64,

    /// Total operations to issue (overrides the default concurrency * 100 formula)
    #[arg(long)]
    total_ops: Option<usize>,

    /// Disable rate limits for raw capacity measurement only (no paired run)
    #[arg(long)]
    no_rate_limit: bool,

    /// Run extended suite including 100k corpus
    #[arg(long)]
    full: bool,

    /// Save results as baseline for future regression comparison
    #[arg(long)]
    save_baseline: bool,

    /// Path to baseline JSON for regression comparison
    #[arg(long)]
    baseline: Option<PathBuf>,

    /// Number of simulated projects (tenants)
    #[arg(long, default_value = "5")]
    num_projects: usize,

    /// Database URL (defaults to DATABASE_URL env var)
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Output directory for reports
    #[arg(long, default_value = "load_test_results")]
    output_dir: PathBuf,
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing for test output visibility
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("warn".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    // Validate rw_ratio early
    let workload_profile = parse_rw_ratio(&cli.rw_ratio)?;

    // Create output directory
    std::fs::create_dir_all(&cli.output_dir)?;

    // ── Database setup ────────────────────────────────────────────────────────

    // Pool size: concurrency + 10, minimum 50
    let max_concurrency = cli.concurrency.unwrap_or(500);
    let pool_size = (max_concurrency + 10).max(50) as u32;

    println!("Connecting to database (pool_size={})...", pool_size);
    let pool = PgPoolOptions::new()
        .max_connections(pool_size)
        .connect(&cli.database_url)
        .await?;

    println!("Running migrations...");
    MIGRATOR.run(&pool).await?;
    println!("Migrations complete.");

    // ── Load baseline if provided ─────────────────────────────────────────────

    let baseline = if let Some(ref baseline_path) = cli.baseline {
        report::load_baseline(baseline_path)?
    } else {
        None
    };

    // ── Determine test matrix ─────────────────────────────────────────────────

    let corpus_sizes: Vec<usize> = if let Some(size) = cli.corpus_size {
        vec![size]
    } else if cli.full {
        vec![100, 1000, 10_000, 100_000]
    } else {
        vec![100, 1000, 10_000]
    };

    let concurrency_levels: Vec<usize> = if let Some(c) = cli.concurrency {
        vec![c]
    } else {
        vec![10, 50, 100, 500]
    };

    let git_sha = report::get_git_sha();

    println!(
        "\nTest matrix: {} corpus size(s) × {} concurrency level(s)",
        corpus_sizes.len(),
        concurrency_levels.len()
    );

    // ── Run tests ─────────────────────────────────────────────────────────────

    let mut all_reports: Vec<LoadTestReport> = Vec::new();

    for &corpus_size in &corpus_sizes {
        println!("\n══════════════════════════════════════");
        println!("Corpus size: {} memories", corpus_size);
        println!("══════════════════════════════════════");

        // Clear and re-seed corpus for this corpus size
        println!("  Clearing corpus...");
        corpus::clear_corpus(&pool).await?;
        println!("  Seeding {} memories across {} projects...", corpus_size, cli.num_projects);
        corpus::seed_corpus(&pool, corpus_size, cli.num_projects).await?;

        for &concurrency in &concurrency_levels {
            let total_ops = cli.total_ops.unwrap_or_else(|| (concurrency * 100).max(1000));

            println!(
                "\n  Concurrency: {} | Ops: {} | RW: {}",
                concurrency, total_ops, cli.rw_ratio
            );

            // Determine which mode combinations to run
            // If --no-rate-limit is set: raw only (no paired run)
            // Otherwise: run both raw (rate limits disabled) then rate-limited
            let modes: Vec<(&str, bool)> = if cli.no_rate_limit {
                vec![("raw", false)]
            } else {
                vec![("raw", false), ("rate-limited", true)]
            };

            for (mode_label, rate_limits_enabled) in &modes {
                println!(
                    "    Running {} workload...",
                    mode_label
                );

                // Build rate limit config
                let rl_config = if *rate_limits_enabled {
                    RateLimitConfig::default()
                } else {
                    RateLimitConfig {
                        enabled: false,
                        ..RateLimitConfig::default()
                    }
                };

                // Build AppState for this test server instance
                let store = PostgresMemoryStore::from_pool(pool.clone()).await?;
                let mut config = Config::default();
                config.rate_limit = rl_config.clone();

                // Use a non-global Prometheus recorder to avoid panic if metrics
                // already initialized in a previous iteration
                let metrics_handle = PrometheusBuilder::new().build_recorder().handle();

                let state = AppState {
                    ready: Arc::new(AtomicBool::new(true)),
                    started_at: Instant::now(),
                    caps: config.resource_caps.clone(),
                    store: Some(Arc::new(store)),
                    config: Arc::new(config),
                    embed_provider: None,
                    embed_sender: None,
                    metrics_handle,
                };

                // Spawn test server on a random port
                let base_url = spawn_test_server(state, &rl_config).await;

                // Build HTTP client with connection pool sized to concurrency
                let http_client = reqwest::Client::builder()
                    .pool_max_idle_per_host(concurrency)
                    .build()?;

                // Build LoadTestConfig
                let load_config = LoadTestConfig {
                    corpus_size,
                    concurrency,
                    rw_ratio: workload_profile.clone(),
                    duration_secs: cli.duration,
                    total_ops,
                    rate_limits_enabled: *rate_limits_enabled,
                    base_url: base_url.clone(),
                    database_url: cli.database_url.clone(),
                };

                // Run the workload and measure wall-clock time
                let wall_start = std::time::Instant::now();
                let results = client::run_workload(&load_config, &http_client).await;
                let elapsed_secs = wall_start.elapsed().as_secs_f64();

                // Aggregate results
                let per_endpoint = lt_metrics::aggregate_results(&results);

                let total_errors: usize = results.iter().filter(|r| r.is_error).count();
                let error_rate = if total_ops > 0 {
                    total_errors as f64 / total_ops as f64
                } else {
                    0.0
                };
                let ops_per_sec = if elapsed_secs > 0.0 {
                    total_ops as f64 / elapsed_secs
                } else {
                    0.0
                };

                // Get search p95 for Fly.io tier recommendation
                let search_p95 = per_endpoint
                    .get("/v1/search")
                    .map(|s| s.p95_ms)
                    .unwrap_or(0);
                let fly_tier = report::recommend_fly_tier(concurrency, search_p95, error_rate);

                // Build the report
                let mut load_report = LoadTestReport {
                    timestamp: Utc::now(),
                    git_sha: git_sha.clone(),
                    mode: mode_label.to_string(),
                    corpus_size,
                    concurrency,
                    rw_ratio: cli.rw_ratio.clone(),
                    duration_secs: elapsed_secs,
                    total_ops,
                    ops_per_sec,
                    error_rate,
                    per_endpoint,
                    baseline_regression: None,
                    fly_tier_recommendation: Some(fly_tier.clone()),
                };

                // Compare against baseline if provided
                if let Some(ref bl) = baseline {
                    let regression = report::compare_baseline(&load_report, bl);
                    load_report.baseline_regression = Some(regression);
                }

                // Save report files
                let report_stem = format!(
                    "corpus{}_concurrency{}_{}",
                    corpus_size, concurrency, mode_label
                );
                let json_path = cli.output_dir.join(format!("{}.json", report_stem));
                let md_path = cli.output_dir.join(format!("{}.md", report_stem));
                report::save_report(&load_report, &json_path, &md_path)?;

                // Print run summary
                println!("    ┌─ Results ─────────────────────────────────");
                println!("    │ Mode:         {}", mode_label);
                println!("    │ Ops/sec:      {:.1}", ops_per_sec);
                println!("    │ Error rate:   {:.2}%", error_rate * 100.0);
                if search_p95 > 0 {
                    println!("    │ Search p95:   {}ms", search_p95);
                }
                println!("    │ Fly.io tier:  {}", fly_tier.lines().next().unwrap_or(&fly_tier));
                println!("    │ Reports:      {}", json_path.display());
                println!("    └───────────────────────────────────────────");

                all_reports.push(load_report);
            }
        }
    }

    // ── Save baseline if requested ─────────────────────────────────────────────

    if cli.save_baseline {
        if let Some(last) = all_reports.last() {
            let baseline_path = cli.output_dir.join("load_test_baseline.json");
            let json = serde_json::to_string_pretty(last)?;
            std::fs::write(&baseline_path, &json)?;
            println!("\nBaseline saved to: {}", baseline_path.display());
        }
    }

    // ── Final summary ──────────────────────────────────────────────────────────

    println!("\n══════════════════════════════════════");
    println!(
        "Complete: {} run(s) finished. Reports in: {}",
        all_reports.len(),
        cli.output_dir.display()
    );
    println!("══════════════════════════════════════");

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Parse the RW ratio string (e.g., "80/20", "50/50", "20/80") into a WorkloadProfile.
fn parse_rw_ratio(s: &str) -> Result<WorkloadProfile> {
    match s {
        "80/20" => Ok(WorkloadProfile::ReadHeavy),
        "50/50" => Ok(WorkloadProfile::Balanced),
        "20/80" => Ok(WorkloadProfile::WriteHeavy),
        _ => Err(anyhow::anyhow!(
            "Invalid --rw-ratio '{}'. Must be one of: 80/20, 50/50, 20/80",
            s
        )),
    }
}

/// Spawn a test HTTP server on a random port. Returns the base URL.
///
/// Mirrors the pattern from crates/memcp-core/tests/api_test.rs.
async fn spawn_test_server(state: AppState, rl_config: &RateLimitConfig) -> String {
    let api_routes = api::router(rl_config);
    let app = Router::new()
        .route("/health", get(memcp::transport::health::status_handler))
        .merge(api_routes)
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind random port");
    let addr = listener.local_addr().expect("get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server error");
    });

    format!("http://{}", addr)
}
