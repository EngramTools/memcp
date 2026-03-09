//! Garbage collection — salience-threshold pruning, TTL expiry, and hard purge.
//!
//! Background worker prunes low-salience memories after configurable age.
//! Soft-delete via deleted_at, hard purge after grace period.
//! Feeds from storage/ (candidate queries), runs as daemon worker via transport/daemon.

pub mod dedup;
pub mod worker;

pub use dedup::{DedupJob, DedupWorker};
pub use worker::run_gc;

/// Result of a GC run.
#[derive(Debug, serde::Serialize)]
pub struct GcResult {
    /// Number of low-salience memories soft-deleted (salience-based pruning).
    pub pruned_count: usize,
    /// Number of TTL-expired memories soft-deleted.
    pub expired_count: usize,
    /// Number of old soft-deleted memories hard-purged.
    pub hard_purged_count: usize,
    /// Reason GC was skipped entirely (e.g. "below floor"), if applicable.
    pub skipped_reason: Option<String>,
    /// Candidate list (populated only in dry-run mode).
    pub candidates: Vec<GcCandidate>,
}

impl GcResult {
    /// Create a skipped result with a reason.
    pub fn skipped(reason: impl Into<String>) -> Self {
        GcResult {
            pruned_count: 0,
            expired_count: 0,
            hard_purged_count: 0,
            skipped_reason: Some(reason.into()),
            candidates: vec![],
        }
    }
}

/// A single GC candidate memory (returned in dry-run mode).
#[derive(Debug, serde::Serialize)]
pub struct GcCandidate {
    /// Memory UUID.
    pub id: String,
    /// First 100 characters of content for identification.
    pub content_snippet: String,
    /// Current FSRS stability score.
    pub stability: f64,
    /// Age of the memory in days.
    pub age_days: i64,
}
