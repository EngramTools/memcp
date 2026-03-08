//! Report generation for load test results.
//!
//! Generates JSON and Markdown output, saves/loads baseline reports for regression
//! comparison, and produces Fly.io tier recommendations based on observed performance.

use std::path::Path;

use anyhow::Result;

use super::{LoadTestReport, RegressionItem, RegressionReport};

// ─── Markdown Report ─────────────────────────────────────────────────────────

/// Generate a human-readable Markdown report from a completed load test run.
pub fn generate_markdown_report(report: &LoadTestReport) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "# Load Test Report — {}\n\n",
        report.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    // Git SHA
    if let Some(ref sha) = report.git_sha {
        out.push_str(&format!("**Git SHA:** `{}`\n\n", sha));
    }

    // Config summary
    out.push_str("## Configuration\n\n");
    out.push_str(&format!("| Parameter | Value |\n|-|-|\n"));
    out.push_str(&format!("| Corpus size | {} memories |\n", report.corpus_size));
    out.push_str(&format!("| Concurrency | {} clients |\n", report.concurrency));
    out.push_str(&format!("| R/W ratio | {} |\n", report.rw_ratio));
    out.push_str(&format!("| Mode | {} |\n", report.mode));
    out.push_str(&format!("| Duration | {:.1}s |\n", report.duration_secs));
    out.push('\n');

    // Overall summary
    out.push_str("## Overall Results\n\n");
    out.push_str(&format!("| Metric | Value |\n|-|-|\n"));
    out.push_str(&format!("| Total ops | {} |\n", report.total_ops));
    out.push_str(&format!("| Throughput | {:.1} ops/sec |\n", report.ops_per_sec));
    out.push_str(&format!("| Error rate | {:.2}% |\n", report.error_rate * 100.0));
    out.push('\n');

    // Per-endpoint table
    out.push_str("## Per-Endpoint Statistics\n\n");
    out.push_str("| Endpoint | Ops | Errors | p50 (ms) | p95 (ms) | p99 (ms) | Mean (ms) | Max (ms) |\n");
    out.push_str("|-|-|-|-|-|-|-|-|\n");

    // Sort endpoints for deterministic output
    let mut endpoints: Vec<(&String, &super::EndpointStats)> =
        report.per_endpoint.iter().collect();
    endpoints.sort_by_key(|(k, _)| k.as_str());

    for (endpoint, stats) in &endpoints {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            endpoint,
            stats.total_ops,
            stats.error_count,
            stats.p50_ms,
            stats.p95_ms,
            stats.p99_ms,
            stats.mean_ms,
            stats.max_ms,
        ));
    }
    out.push('\n');

    // Fly.io tier recommendation
    if let Some(ref tier) = report.fly_tier_recommendation {
        out.push_str("## Fly.io Tier Recommendation\n\n");
        out.push_str(tier);
        out.push_str("\n\n");
    }

    // Baseline regression section
    if let Some(ref regression) = report.baseline_regression {
        out.push_str("## Baseline Regression Analysis\n\n");
        out.push_str(&format!(
            "Compared against baseline from: {}\n\n",
            regression.baseline_date.format("%Y-%m-%d %H:%M:%S UTC")
        ));

        let flagged_count = regression.regressions.iter().filter(|r| r.flagged).count();
        if flagged_count > 0 {
            out.push_str(&format!(
                "**{} regression(s) detected** (p95 latency increased >20%%)\n\n",
                flagged_count
            ));
        } else {
            out.push_str("No significant regressions detected.\n\n");
        }

        if !regression.regressions.is_empty() {
            out.push_str("| Endpoint | Baseline p95 (ms) | Current p95 (ms) | Change | Flagged |\n");
            out.push_str("|-|-|-|-|-|\n");

            let mut items = regression.regressions.clone();
            items.sort_by_key(|r| r.endpoint.clone());

            for item in &items {
                let flag_str = if item.flagged { "YES" } else { "no" };
                out.push_str(&format!(
                    "| {} | {} | {} | {:.1}% | {} |\n",
                    item.endpoint,
                    item.baseline_p95_ms,
                    item.current_p95_ms,
                    item.change_pct,
                    flag_str,
                ));
            }
            out.push('\n');
        }
    }

    out
}

// ─── Save / Load ─────────────────────────────────────────────────────────────

/// Write the report to `json_path` as pretty-printed JSON and to `md_path` as Markdown.
pub fn save_report(report: &LoadTestReport, json_path: &Path, md_path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| anyhow::anyhow!("Failed to serialize report: {}", e))?;
    std::fs::write(json_path, &json)
        .map_err(|e| anyhow::anyhow!("Failed to write JSON report to {:?}: {}", json_path, e))?;

    let md = generate_markdown_report(report);
    std::fs::write(md_path, &md)
        .map_err(|e| anyhow::anyhow!("Failed to write Markdown report to {:?}: {}", md_path, e))?;

    Ok(())
}

/// Load a previously saved baseline report from a JSON file.
///
/// Returns `None` if the file does not exist. Returns an error for parse failures.
pub fn load_baseline(path: &Path) -> Result<Option<LoadTestReport>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read baseline {:?}: {}", path, e))?;
    let report: LoadTestReport = serde_json::from_str(&json)
        .map_err(|e| anyhow::anyhow!("Failed to parse baseline {:?}: {}", path, e))?;
    Ok(Some(report))
}

// ─── Regression Comparison ────────────────────────────────────────────────────

/// Compare the current report against a baseline and produce a `RegressionReport`.
///
/// For each endpoint present in both reports, computes the percentage change in p95
/// latency. Endpoints present only in the current run are included with `change_pct = 0.0`
/// and `flagged = false`. Endpoints only in the baseline are omitted.
///
/// Flags the endpoint when `change_pct > 20.0` (>20% increase in p95 latency).
pub fn compare_baseline(current: &LoadTestReport, baseline: &LoadTestReport) -> RegressionReport {
    let mut regressions: Vec<RegressionItem> = Vec::new();

    for (endpoint, current_stats) in &current.per_endpoint {
        if let Some(baseline_stats) = baseline.per_endpoint.get(endpoint) {
            let change_pct = if baseline_stats.p95_ms == 0 {
                0.0
            } else {
                (current_stats.p95_ms as f64 - baseline_stats.p95_ms as f64)
                    / baseline_stats.p95_ms as f64
                    * 100.0
            };

            regressions.push(RegressionItem {
                endpoint: endpoint.clone(),
                baseline_p95_ms: baseline_stats.p95_ms,
                current_p95_ms: current_stats.p95_ms,
                change_pct,
                flagged: change_pct > 20.0,
            });
        }
    }

    RegressionReport {
        baseline_date: baseline.timestamp,
        regressions,
    }
}

// ─── Fly.io Tier Recommendation ───────────────────────────────────────────────

/// Recommend a Fly.io machine tier based on observed concurrency and search performance.
///
/// Tier table (from RESEARCH.md):
/// - Entry   (shared-cpu-1x):    concurrency <= 10,  p95 < 200ms, errors < 1%
/// - Starter (shared-cpu-2x):    concurrency <= 50
/// - Growth  (shared-cpu-4x):    concurrency <= 100
/// - Launch  (performance-2x):   concurrency <= 200
/// - Scale   (performance-4x):   concurrency <= 500
/// - Enterprise (performance-8x): concurrency > 500
///
/// Adjusts upward if p95 > 500ms or error_rate > 5% at a given tier.
pub fn recommend_fly_tier(concurrency: usize, p95_search_ms: u64, error_rate: f64) -> String {
    let degraded = p95_search_ms > 500 || error_rate > 0.05;

    let (tier, machine, pg_tier, pg_connections) = if concurrency <= 10 && !degraded {
        ("Entry", "shared-cpu-1x", "Hobby", 25)
    } else if concurrency <= 50 && !degraded {
        ("Starter", "shared-cpu-2x", "Basic", 50)
    } else if concurrency <= 100 && !degraded {
        ("Growth", "shared-cpu-4x", "Standard-2", 100)
    } else if concurrency <= 200 && !degraded {
        ("Launch", "performance-2x", "Standard-4", 200)
    } else if concurrency <= 500 && !degraded {
        ("Scale", "performance-4x", "Standard-8", 500)
    } else {
        ("Enterprise", "performance-8x", "Standard-16", 1000)
    };

    // If degraded at chosen tier, bump up one level
    let (tier, machine, pg_tier, pg_connections) = if degraded {
        // Apply upward adjustment
        if concurrency <= 10 {
            ("Starter", "shared-cpu-2x", "Basic", 50usize)
        } else if concurrency <= 50 {
            ("Growth", "shared-cpu-4x", "Standard-2", 100)
        } else if concurrency <= 100 {
            ("Launch", "performance-2x", "Standard-4", 200)
        } else if concurrency <= 200 {
            ("Scale", "performance-4x", "Standard-8", 500)
        } else {
            ("Enterprise", "performance-8x", "Standard-16", 1000)
        }
    } else {
        (tier, machine, pg_tier, pg_connections)
    };

    format!(
        "{} ({machine})\n\nRequires Fly Postgres {pg_tier} ({pg_connections} connection limit)",
        tier,
        machine = machine,
        pg_tier = pg_tier,
        pg_connections = pg_connections,
    )
}

// ─── Git SHA ──────────────────────────────────────────────────────────────────

/// Capture the current git short SHA via `git rev-parse --short HEAD`.
///
/// Returns `None` if git is not available, the working directory is not a git repo,
/// or any other error occurs.
pub fn get_git_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
    } else {
        None
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_test::{EndpointStats, LoadTestReport, RegressionReport};
    use std::collections::HashMap;

    fn make_report(concurrency: usize, p95: u64, error_rate: f64) -> LoadTestReport {
        let mut per_endpoint = HashMap::new();
        per_endpoint.insert(
            "/v1/search".to_string(),
            EndpointStats {
                total_ops: 1000,
                error_count: (1000.0 * error_rate) as usize,
                p50_ms: p95 / 2,
                p95_ms: p95,
                p99_ms: p95 + 20,
                mean_ms: p95 / 2,
                max_ms: p95 + 100,
            },
        );
        LoadTestReport {
            timestamp: chrono::Utc::now(),
            git_sha: Some("abc1234".to_string()),
            mode: "raw".to_string(),
            corpus_size: 10_000,
            concurrency,
            rw_ratio: "80/20".to_string(),
            duration_secs: 60.0,
            total_ops: 1000,
            ops_per_sec: 1000.0 / 60.0,
            error_rate,
            per_endpoint,
            baseline_regression: None,
            fly_tier_recommendation: None,
        }
    }

    #[test]
    fn test_markdown_report_contains_header() {
        let report = make_report(50, 150, 0.01);
        let md = generate_markdown_report(&report);
        assert!(md.contains("# Load Test Report"));
        assert!(md.contains("abc1234"));
        assert!(md.contains("/v1/search"));
        assert!(md.contains("80/20"));
    }

    #[test]
    fn test_save_and_load_baseline() {
        let report = make_report(50, 150, 0.01);
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("report.json");
        let md_path = dir.path().join("report.md");

        save_report(&report, &json_path, &md_path).expect("save_report failed");
        assert!(json_path.exists());
        assert!(md_path.exists());

        let loaded = load_baseline(&json_path).expect("load_baseline failed");
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.concurrency, 50);
        assert_eq!(loaded.corpus_size, 10_000);
    }

    #[test]
    fn test_load_baseline_returns_none_for_missing_file() {
        let result = load_baseline(Path::new("/nonexistent/path/baseline.json"))
            .expect("should return Ok(None)");
        assert!(result.is_none());
    }

    #[test]
    fn test_compare_baseline_flags_regression() {
        let baseline = make_report(50, 100, 0.01);  // p95 = 100ms
        let current = make_report(50, 130, 0.01);   // p95 = 130ms → 30% increase → flagged

        let regression = compare_baseline(&current, &baseline);
        let item = regression.regressions.iter().find(|r| r.endpoint == "/v1/search").unwrap();
        assert!(item.flagged, "30% regression should be flagged");
        assert!((item.change_pct - 30.0).abs() < 1.0);
    }

    #[test]
    fn test_compare_baseline_no_regression() {
        let baseline = make_report(50, 100, 0.01);  // p95 = 100ms
        let current = make_report(50, 115, 0.01);   // p95 = 115ms → 15% increase → not flagged

        let regression = compare_baseline(&current, &baseline);
        let item = regression.regressions.iter().find(|r| r.endpoint == "/v1/search").unwrap();
        assert!(!item.flagged, "15% change should not be flagged");
    }

    #[test]
    fn test_fly_tier_entry() {
        let tier = recommend_fly_tier(5, 100, 0.001);
        assert!(tier.contains("Entry"));
        assert!(tier.contains("shared-cpu-1x"));
    }

    #[test]
    fn test_fly_tier_scale() {
        let tier = recommend_fly_tier(300, 200, 0.01);
        assert!(tier.contains("Scale"));
        assert!(tier.contains("performance-4x"));
    }

    #[test]
    fn test_fly_tier_bumps_up_on_high_p95() {
        // p95 > 500ms at concurrency=10 should bump from Entry to Starter
        let tier = recommend_fly_tier(10, 600, 0.001);
        assert!(tier.contains("Starter"));
    }

    #[test]
    fn test_get_git_sha_returns_something() {
        // In a git repo, this should return Some. We just check it doesn't panic.
        let _sha = get_git_sha();
    }
}
