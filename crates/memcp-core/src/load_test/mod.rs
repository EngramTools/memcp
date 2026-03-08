//! Load test library foundation.
//!
//! Provides type definitions, corpus seeding, latency metrics computation,
//! and report generation for the memcp load test driver.
//!
//! Modules:
//! - `corpus`: Seeds a Postgres database with synthetic memories + embeddings
//! - `metrics`: Aggregates raw RequestResult data into per-endpoint stats
//! - `report`: Generates JSON + Markdown reports with baseline regression detection

pub mod corpus;
pub mod metrics;
pub mod report;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for a single load test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestConfig {
    /// Number of memories seeded into Postgres before the run (100, 1000, 10000, 100000).
    pub corpus_size: usize,
    /// Number of concurrent HTTP clients issuing requests (10, 50, 100, 500).
    pub concurrency: usize,
    /// Read/write operation ratio.
    pub rw_ratio: WorkloadProfile,
    /// Maximum test duration in seconds (hard timeout).
    pub duration_secs: u64,
    /// Total operations to issue across all clients.
    pub total_ops: usize,
    /// When true, rate limiting is disabled — measures raw server capacity.
    /// When false, measures production behavior with GovernorLayer enabled.
    pub rate_limits_enabled: bool,
    /// Base URL of the running memcp HTTP server (e.g., "http://localhost:3000").
    pub base_url: String,
    /// Postgres connection URL for corpus seeding (e.g., "postgres://memcp:memcp@localhost:5433/memcp").
    pub database_url: String,
}

// ─── Workload Profile ─────────────────────────────────────────────────────────

/// Defines the read/write operation mix for a load test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkloadProfile {
    /// 80% read (search/recall), 20% write (store) — label "80/20".
    ReadHeavy,
    /// 50% read, 50% write — label "50/50".
    Balanced,
    /// 20% read, 80% write — label "20/80".
    WriteHeavy,
}

impl WorkloadProfile {
    /// Returns the percentage of operations that should be writes.
    pub fn write_pct(&self) -> usize {
        match self {
            WorkloadProfile::ReadHeavy => 20,
            WorkloadProfile::Balanced => 50,
            WorkloadProfile::WriteHeavy => 80,
        }
    }

    /// Returns a human-readable label for the profile.
    pub fn label(&self) -> &'static str {
        match self {
            WorkloadProfile::ReadHeavy => "80/20",
            WorkloadProfile::Balanced => "50/50",
            WorkloadProfile::WriteHeavy => "20/80",
        }
    }
}

// ─── Results ──────────────────────────────────────────────────────────────────

/// Per-endpoint latency and error statistics for a completed load test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointStats {
    /// Total number of operations issued to this endpoint.
    pub total_ops: usize,
    /// Number of operations that returned a non-2xx status or connection error.
    pub error_count: usize,
    /// 50th percentile latency in milliseconds.
    pub p50_ms: u64,
    /// 95th percentile latency in milliseconds.
    pub p95_ms: u64,
    /// 99th percentile latency in milliseconds.
    pub p99_ms: u64,
    /// Mean latency across all operations in milliseconds.
    pub mean_ms: u64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: u64,
}

/// Full report produced at the end of a load test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadTestReport {
    /// UTC timestamp when the test completed.
    pub timestamp: DateTime<Utc>,
    /// Git short SHA captured at test start (`git rev-parse --short HEAD`), if available.
    pub git_sha: Option<String>,
    /// Test mode: "raw" (rate limits disabled) or "rate-limited" (production config).
    pub mode: String,
    /// Number of memories seeded into the corpus.
    pub corpus_size: usize,
    /// Number of concurrent HTTP clients.
    pub concurrency: usize,
    /// R/W ratio label (e.g., "80/20", "50/50", "20/80").
    pub rw_ratio: String,
    /// Actual test wall-clock duration in seconds.
    pub duration_secs: f64,
    /// Total operations completed (successful + errored).
    pub total_ops: usize,
    /// Throughput: total_ops / duration_secs.
    pub ops_per_sec: f64,
    /// Fraction of operations that returned errors (0.0–1.0).
    pub error_rate: f64,
    /// Per-endpoint statistics keyed by endpoint path (e.g., "/v1/store").
    pub per_endpoint: HashMap<String, EndpointStats>,
    /// Regression report vs a saved baseline, if baseline was loaded.
    pub baseline_regression: Option<RegressionReport>,
    /// Fly.io tier recommendation based on observed performance.
    pub fly_tier_recommendation: Option<String>,
}

// ─── Regression ───────────────────────────────────────────────────────────────

/// Summary of regressions detected when comparing to a saved baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReport {
    /// Timestamp of the baseline report this was compared against.
    pub baseline_date: DateTime<Utc>,
    /// Individual endpoint regression items (only endpoints present in both reports).
    pub regressions: Vec<RegressionItem>,
}

/// Regression data for a single endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionItem {
    /// Endpoint path (e.g., "/v1/search").
    pub endpoint: String,
    /// Baseline p95 latency in milliseconds.
    pub baseline_p95_ms: u64,
    /// Current p95 latency in milliseconds.
    pub current_p95_ms: u64,
    /// Percentage change in p95 latency (positive = slower, negative = faster).
    pub change_pct: f64,
    /// True when `change_pct > 20.0` — indicates a significant regression.
    pub flagged: bool,
}
