/// Garbage collection module for memory hygiene.
///
/// Provides salience-based pruning, TTL expiry, dry-run support, and hard purge.
/// GC runs both automatically (daemon schedule) and on-demand (CLI `memcp gc`).

pub mod worker;

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
