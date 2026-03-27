//! Entity normalization pipeline.
//!
//! Reads memories where extraction is complete but `entity_normalization_status = 'pending'`,
//! resolves extracted entity strings against the canonical `entities` table, creates
//! `entity_mentions` links, stores parsed facts as `entity_facts`, and marks each
//! memory `complete` (or `failed` after 3 retries).

pub mod worker;

pub use worker::NormalizationWorker;

/// Result of normalizing entities for a single memory.
#[derive(Debug, Clone)]
pub struct NormalizationResult {
    /// Number of entities upserted or matched.
    pub entities_resolved: usize,
    /// Number of mentions created.
    pub mentions_created: usize,
    /// Number of facts stored.
    pub facts_stored: usize,
}

/// A pending normalization job for a memory.
#[derive(Debug, Clone)]
pub struct NormalizationJob {
    /// The memory ID to normalize entities for.
    pub memory_id: String,
    /// Raw entity strings extracted from the memory.
    pub extracted_entities: Vec<String>,
    /// Raw fact strings extracted from the memory (backward-compat flat format).
    pub extracted_facts: Vec<String>,
    /// Structured facts with entity linkage. Empty when not available (old format).
    pub structured_facts: Vec<crate::pipeline::extraction::StructuredFact>,
    /// Full text content of the memory (for context snippets).
    pub content: String,
    /// Current attempt number (for retry logic).
    pub attempt: u8,
}
