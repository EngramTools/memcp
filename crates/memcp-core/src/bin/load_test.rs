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
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use axum::Router;
use axum::routing::get;
use clap::Parser;
use chrono::Utc;
use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::time::Instant;

use memcp::MIGRATOR;
use memcp::config::{Config, CurationConfig, RateLimitConfig};
use memcp::load_test::{LoadTestConfig, LoadTestReport, SecurityReport, TrustCorpusConfig, TrustWorkloadResult, WorkloadProfile};
use memcp::load_test::{client, corpus, metrics as lt_metrics, report};
use memcp::load_test::trust::{self, MockLlmProvider, TrustWorkloadState};
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

    /// Workload profile: "standard" (existing R/W test) or "trust" (security correctness)
    #[arg(long, default_value = "standard")]
    profile: String,

    /// Use real Ollama LLM instead of mock provider for curation (trust profile only)
    #[arg(long)]
    real_llm: bool,
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

    // ── Profile routing ──────────────────────────────────────────────────────
    match cli.profile.as_str() {
        "trust" => {
            return run_trust_workload_cli(&cli, &pool).await;
        }
        "standard" | _ => {
            // existing flow below
        }
    }

    // Validate rw_ratio (only needed for standard profile)
    let workload_profile = parse_rw_ratio(&cli.rw_ratio)?;

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

/// Run the trust workload: seed → curate → audit → report.
async fn run_trust_workload_cli(cli: &Cli, pool: &PgPool) -> Result<()> {
    let corpus_size = cli.corpus_size.unwrap_or(1000);
    let concurrency = cli.concurrency.unwrap_or(50);

    let trust_config = TrustCorpusConfig {
        corpus_size,
        num_projects: cli.num_projects,
        poison_ratio: 0.05,
    };

    println!("\n== Trust Workload ==");
    println!("Corpus size: {} ({} poisoned)", corpus_size, trust_config.poison_count());
    println!("Concurrency: {}", concurrency);
    println!("Real LLM: {}", cli.real_llm);

    // 1. Clear and seed trust corpus
    println!("\nClearing corpus...");
    corpus::clear_corpus(pool).await?;

    // Also clear curation_runs to avoid windowing issues
    sqlx::query("TRUNCATE TABLE curation_runs CASCADE")
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("TRUNCATE curation_runs failed: {}", e))?;

    println!("Seeding trust corpus...");
    let corpus_result = corpus::seed_trust_corpus(pool, &trust_config).await?;

    // 2. Initialize workload state
    let state = Arc::new(TrustWorkloadState::new());
    {
        let mut poisoned = state.poisoned_ids.write().await;
        for id in corpus_result.poisoned_ids.keys() {
            poisoned.insert(id.clone());
        }
    }

    // 3. Create store and curation config
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await?);
    let curation_config = CurationConfig {
        enabled: true,
        max_candidates_per_run: 500,
        ..CurationConfig::default()
    };

    // 4. Create provider (mock or real)
    let _mock_latency = if cli.real_llm { 0u64 } else { 200u64 };

    // 5. Spawn curation loop
    let shutdown = Arc::new(AtomicBool::new(false));
    let curation_lock = Arc::new(tokio::sync::Mutex::new(()));

    let curation_handle = {
        let store_clone = store.clone();
        let config_clone = curation_config.clone();
        let lock_clone = curation_lock.clone();
        let state_clone = state.clone();
        let shutdown_clone = shutdown.clone();
        let mock_latency = _mock_latency;

        tokio::spawn(async move {
            let provider = MockLlmProvider::new(mock_latency);
            trust::run_curation_loop(
                store_clone,
                config_clone,
                &provider,
                lock_clone,
                state_clone,
                5, // 5-second interval
                shutdown_clone,
            )
            .await;
        })
    };

    // 6. Optionally run HTTP workload concurrently
    let duration_secs = cli.duration;
    println!("\nRunning trust workload for {}s with curation loop...", duration_secs);

    // Run HTTP workload if we have a server
    let rl_config = RateLimitConfig {
        enabled: false,
        ..RateLimitConfig::default()
    };
    let app_store = PostgresMemoryStore::from_pool(pool.clone()).await?;
    let mut config = Config::default();
    config.rate_limit = rl_config.clone();
    let metrics_handle = PrometheusBuilder::new().build_recorder().handle();

    let app_state = AppState {
        ready: Arc::new(AtomicBool::new(true)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(app_store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle,
    };

    let base_url = spawn_test_server(app_state, &rl_config).await;

    let http_client = reqwest::Client::builder()
        .pool_max_idle_per_host(concurrency)
        .build()?;

    let total_ops = cli.total_ops.unwrap_or_else(|| (concurrency * 100).max(1000));

    let load_config = LoadTestConfig {
        corpus_size,
        concurrency,
        rw_ratio: parse_rw_ratio(&cli.rw_ratio).unwrap_or(WorkloadProfile::ReadHeavy),
        duration_secs,
        total_ops,
        rate_limits_enabled: false,
        base_url,
        database_url: cli.database_url.clone(),
    };

    let wall_start = std::time::Instant::now();
    let results = client::run_workload(&load_config, &http_client).await;
    let elapsed_secs = wall_start.elapsed().as_secs_f64();

    // 7. Signal curation shutdown and wait
    shutdown.store(true, Ordering::Relaxed);
    let _ = curation_handle.await;

    // 8. Post-run audit
    println!("\nRunning post-run audit...");
    let audit_violations = trust::post_run_audit(pool, &state).await?;
    {
        let mut violations = state.violations.lock().await;
        violations.extend(audit_violations);
    }

    // 9. Build security report
    let curation_metrics = state.curation_metrics.lock().await;
    let violations = state.violations.lock().await;
    let quarantined = state.quarantined_ids.read().await;
    let poisoned = state.poisoned_ids.read().await;

    let detection_rate = if poisoned.len() > 0 {
        quarantined.len() as f64 / poisoned.len() as f64
    } else {
        0.0
    };

    let security_report = SecurityReport {
        poisoned_seeded: poisoned.len(),
        quarantined_count: quarantined.len(),
        detection_rate,
        false_positive_count: 0, // Would need tracking clean IDs that got flagged
        violations: violations.iter().map(|v| {
            format!("{:?}: memory={} expected={} actual={}",
                v.violation_type, v.memory_id, v.expected, v.actual)
        }).collect(),
        curation_cycles: curation_metrics.cycle_count,
        p1_drain_ms: curation_metrics.p1_drain_ms.clone(),
        p2_drain_ms: curation_metrics.p2_drain_ms.clone(),
        normal_drain_ms: curation_metrics.normal_drain_ms.clone(),
        dwell_times_ms: curation_metrics.dwell_times_ms.clone(),
    };

    // 10. Build standard report
    let per_endpoint = lt_metrics::aggregate_results(&results);
    let total_errors: usize = results.iter().filter(|r| r.is_error).count();
    let error_rate = if total_ops > 0 { total_errors as f64 / total_ops as f64 } else { 0.0 };
    let ops_per_sec = if elapsed_secs > 0.0 { total_ops as f64 / elapsed_secs } else { 0.0 };

    let load_report = LoadTestReport {
        timestamp: Utc::now(),
        git_sha: report::get_git_sha(),
        mode: "trust".to_string(),
        corpus_size,
        concurrency,
        rw_ratio: cli.rw_ratio.clone(),
        duration_secs: elapsed_secs,
        total_ops,
        ops_per_sec,
        error_rate,
        per_endpoint,
        baseline_regression: None,
        fly_tier_recommendation: None,
    };

    // 11. Generate and save report
    let mut md = report::generate_markdown_report(&load_report);
    md.push_str(&report::generate_security_section(&security_report));

    let report_stem = format!("trust_corpus{}_concurrency{}", corpus_size, concurrency);
    let json_path = cli.output_dir.join(format!("{}.json", report_stem));
    let md_path = cli.output_dir.join(format!("{}.md", report_stem));

    report::save_report(&load_report, &json_path, &md_path)?;
    // Also save the full report with security section
    std::fs::write(&md_path, &md)?;

    // 12. Print summary
    println!("\n== Trust Workload Results ==");
    println!("Poisoned seeded:     {}", security_report.poisoned_seeded);
    println!("Quarantined:         {}", security_report.quarantined_count);
    println!("Detection rate:      {:.1}%", security_report.detection_rate * 100.0);
    println!("Violations:          {}", security_report.violations.len());
    println!("Curation cycles:     {}", security_report.curation_cycles);
    println!("HTTP ops/sec:        {:.1}", ops_per_sec);
    println!("Reports saved to:    {}", cli.output_dir.display());

    let _result = TrustWorkloadResult {
        security_report,
        standard_report: Some(load_report),
    };

    Ok(())
}
