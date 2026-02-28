//! Embedding promotion sweep — promotes important memories from fast to quality tier.
//!
//! Periodically evaluates memories embedded with the fast (local) model and re-embeds
//! those that have proven important (high stability, multiple reinforcements) with the
//! quality (API) model for better retrieval accuracy.

pub mod worker;

/// Result of a single promotion sweep cycle.
#[derive(Debug)]
pub struct PromotionResult {
    /// Number of memories successfully promoted in this sweep
    pub promoted_count: usize,
    /// Number of candidates evaluated
    pub candidates_evaluated: usize,
    /// Number of failed promotions (embedding API errors)
    pub failed_count: usize,
    /// If the sweep was skipped, the reason why
    pub skipped_reason: Option<String>,
}
