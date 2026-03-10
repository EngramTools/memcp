//! Search quality benchmark harness (LongMemEval).
//!
//! Provides dataset ingestion, search evaluation, and reporting for
//! benchmarking the memcp search pipeline. Used by the benchmark binary.

pub mod dataset;
pub mod evaluate;
pub mod ingest;
pub mod locomo;
pub mod prompts;
pub mod report;
pub mod runner;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for a benchmark run. Controls search weights and QI features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkConfig {
    pub name: String,
    pub bm25_weight: f64,
    pub vector_weight: f64,
    pub symbolic_weight: f64,
    pub qi_expansion: bool,
    pub qi_reranking: bool,
}

/// Predefined configurations for comparison runs.
pub fn default_configs() -> Vec<BenchmarkConfig> {
    vec![
        BenchmarkConfig {
            name: "vector-only".into(),
            bm25_weight: 0.0,
            vector_weight: 1.0,
            symbolic_weight: 0.0,
            qi_expansion: false,
            qi_reranking: false,
        },
        BenchmarkConfig {
            name: "hybrid".into(),
            bm25_weight: 1.0,
            vector_weight: 1.0,
            symbolic_weight: 1.0,
            qi_expansion: false,
            qi_reranking: false,
        },
        BenchmarkConfig {
            name: "hybrid+qi".into(),
            bm25_weight: 1.0,
            vector_weight: 1.0,
            symbolic_weight: 1.0,
            qi_expansion: true,
            qi_reranking: true,
        },
    ]
}

/// Result for a single benchmark question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResult {
    pub question_id: String,
    pub question_type: String,
    pub is_abstention: bool,
    pub correct: bool,
    pub hypothesis: String,
    pub ground_truth: String,
    pub retrieved_count: usize,
    pub latency_ms: u64,
    /// How many of the evidence (answer) sessions were found in retrieved memories.
    pub evidence_sessions_found: usize,
    /// Total number of evidence sessions for this question.
    pub evidence_sessions_total: usize,
}

/// Checkpoint state for resumable benchmark runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkState {
    pub config_name: String,
    pub completed_question_ids: Vec<String>,
    pub results: Vec<QuestionResult>,
    pub started_at: DateTime<Utc>,
}

/// Check if a database URL looks like a production database.
/// Returns true if the URL contains suspicious indicators (cloud provider domains,
/// "prod"/"production" in hostname or path).
pub fn check_database_url_safety(url: &str) -> bool {
    let suspicious = [
        "prod",
        "production",
        "rds.amazonaws.com",
        "neon.tech",
        "supabase.co",
        "fly.dev",
        ".railway.app",
        "aiven.io",
    ];
    let lower = url.to_lowercase();
    suspicious.iter().any(|s| lower.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_safety_safe() {
        assert!(!check_database_url_safety(
            "postgres://localhost:5433/memcp"
        ));
        assert!(!check_database_url_safety(
            "postgres://memcp:memcp@localhost:5433/memcp"
        ));
        assert!(!check_database_url_safety(
            "postgres://user@10.0.0.1:5432/mydb"
        ));
    }

    #[test]
    fn test_url_safety_rds() {
        assert!(check_database_url_safety(
            "postgres://prod-db.rds.amazonaws.com/mydb"
        ));
    }

    #[test]
    fn test_url_safety_neon() {
        assert!(check_database_url_safety(
            "postgres://user@db.neon.tech/mydb"
        ));
    }

    #[test]
    fn test_url_safety_supabase() {
        assert!(check_database_url_safety(
            "postgres://user@db.supabase.co/mydb"
        ));
    }

    #[test]
    fn test_url_safety_fly() {
        assert!(check_database_url_safety(
            "postgres://user@db.fly.dev/mydb"
        ));
    }

    #[test]
    fn test_url_safety_production_keyword() {
        assert!(check_database_url_safety(
            "postgres://user@myhost/production"
        ));
        assert!(check_database_url_safety(
            "postgres://prod-server:5432/mydb"
        ));
    }
}
