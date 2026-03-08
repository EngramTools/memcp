//! Latency aggregation and percentile computation for load test results.
//!
//! The actual HTTP timing happens in the client driver (Plan 02). This module
//! receives completed `RequestResult` values and computes per-endpoint statistics.

use std::collections::HashMap;

use super::{EndpointStats};

// ─── Raw Result ───────────────────────────────────────────────────────────────

/// A single completed HTTP request during the load test.
///
/// Collected by the concurrent client driver and passed to `aggregate_results`
/// for statistical summarization.
#[derive(Debug, Clone)]
pub struct RequestResult {
    /// HTTP endpoint path (e.g., "/v1/store", "/v1/search", "/v1/recall").
    pub endpoint: String,
    /// HTTP response status code (0 if connection failed before receiving a response).
    pub status: u16,
    /// Round-trip latency in milliseconds (wall-clock from request send to response complete).
    pub latency_ms: u64,
    /// True when status >= 400 or when a connection-level error occurred.
    pub is_error: bool,
}

// ─── Percentile Computation ───────────────────────────────────────────────────

/// Compute p50, p95, and p99 latency percentiles from a sorted (or unsorted) latency slice.
///
/// Sorts the input in place for efficiency. Returns `(0, 0, 0)` for an empty vec.
/// Uses ceiling-index method: index = ceil(p * n) - 1, clamped to [0, n-1].
///
/// # Example
/// ```
/// let mut latencies = vec![10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100];
/// let (p50, p95, p99) = memcp::load_test::metrics::compute_percentiles(&mut latencies);
/// assert_eq!(p50, 50);
/// assert_eq!(p95, 95); // or 100 depending on rounding
/// ```
pub fn compute_percentiles(latencies: &mut Vec<u64>) -> (u64, u64, u64) {
    if latencies.is_empty() {
        return (0, 0, 0);
    }

    latencies.sort_unstable();
    let n = latencies.len();

    let idx_at = |pct: f64| -> usize {
        let raw = (pct * n as f64).ceil() as usize;
        raw.saturating_sub(1).min(n - 1)
    };

    let p50 = latencies[idx_at(0.50)];
    let p95 = latencies[idx_at(0.95)];
    let p99 = latencies[idx_at(0.99)];

    (p50, p95, p99)
}

// ─── Aggregation ─────────────────────────────────────────────────────────────

/// Aggregate a slice of `RequestResult` values into per-endpoint `EndpointStats`.
///
/// Groups results by `endpoint`, then computes percentiles, mean, max, and error count
/// for each group. Endpoints with zero results are not included in the output.
pub fn aggregate_results(results: &[RequestResult]) -> HashMap<String, EndpointStats> {
    // Group latencies and error counts by endpoint
    let mut groups: HashMap<String, (Vec<u64>, usize)> = HashMap::new();

    for r in results {
        let entry = groups.entry(r.endpoint.clone()).or_insert_with(|| (Vec::new(), 0));
        entry.0.push(r.latency_ms);
        if r.is_error {
            entry.1 += 1;
        }
    }

    // Compute stats for each endpoint
    groups
        .into_iter()
        .map(|(endpoint, (mut latencies, error_count))| {
            let total_ops = latencies.len();

            let mean_ms = if total_ops > 0 {
                latencies.iter().sum::<u64>() / total_ops as u64
            } else {
                0
            };

            let max_ms = latencies.iter().copied().max().unwrap_or(0);

            let (p50_ms, p95_ms, p99_ms) = compute_percentiles(&mut latencies);

            let stats = EndpointStats {
                total_ops,
                error_count,
                p50_ms,
                p95_ms,
                p99_ms,
                mean_ms,
                max_ms,
            };

            (endpoint, stats)
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_percentiles_empty() {
        let (p50, p95, p99) = compute_percentiles(&mut vec![]);
        assert_eq!((p50, p95, p99), (0, 0, 0));
    }

    #[test]
    fn test_compute_percentiles_single() {
        let (p50, p95, p99) = compute_percentiles(&mut vec![42]);
        assert_eq!((p50, p95, p99), (42, 42, 42));
    }

    #[test]
    fn test_compute_percentiles_ten_values() {
        // [10, 20, 30, 40, 50, 60, 70, 80, 90, 100] — already sorted
        let mut latencies: Vec<u64> = (1..=10).map(|i| i * 10).collect();
        let (p50, p95, p99) = compute_percentiles(&mut latencies);
        // p50: ceil(0.50*10)=5, idx=4 → 50
        assert_eq!(p50, 50);
        // p95: ceil(0.95*10)=10, idx=9 → 100
        assert_eq!(p95, 100);
        // p99: ceil(0.99*10)=10, idx=9 → 100
        assert_eq!(p99, 100);
    }

    #[test]
    fn test_compute_percentiles_unsorted_input() {
        let mut latencies = vec![100u64, 10, 50, 30, 70, 20, 90, 40, 60, 80];
        let (p50, p95, p99) = compute_percentiles(&mut latencies);
        // After sort: [10,20,30,40,50,60,70,80,90,100]
        assert_eq!(p50, 50);
        assert_eq!(p95, 100);
        assert_eq!(p99, 100);
    }

    #[test]
    fn test_aggregate_results_groups_by_endpoint() {
        let results = vec![
            RequestResult { endpoint: "/v1/store".into(), status: 200, latency_ms: 10, is_error: false },
            RequestResult { endpoint: "/v1/store".into(), status: 200, latency_ms: 20, is_error: false },
            RequestResult { endpoint: "/v1/search".into(), status: 200, latency_ms: 50, is_error: false },
            RequestResult { endpoint: "/v1/search".into(), status: 500, latency_ms: 100, is_error: true },
        ];

        let stats = aggregate_results(&results);

        assert_eq!(stats.len(), 2);

        let store_stats = stats.get("/v1/store").expect("store stats missing");
        assert_eq!(store_stats.total_ops, 2);
        assert_eq!(store_stats.error_count, 0);
        assert_eq!(store_stats.mean_ms, 15);
        assert_eq!(store_stats.max_ms, 20);

        let search_stats = stats.get("/v1/search").expect("search stats missing");
        assert_eq!(search_stats.total_ops, 2);
        assert_eq!(search_stats.error_count, 1);
        assert_eq!(search_stats.max_ms, 100);
    }

    #[test]
    fn test_aggregate_results_empty() {
        let stats = aggregate_results(&[]);
        assert!(stats.is_empty());
    }
}
