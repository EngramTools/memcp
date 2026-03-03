//! Security helpers for the import pipeline.
//!
//! Provides:
//! - Prompt injection detection and flagging
//! - ZIP bomb protection constants (used by ZIP readers)

use super::ImportChunk;

/// Patterns that indicate prompt injection attempts.
/// Detection is case-insensitive substring match.
pub const INJECTION_PATTERNS: &[&str] = &[
    "ignore previous instructions",
    "ignore all previous",
    "disregard previous",
    "system:",
    "you are now",
    "new instructions:",
    "forget everything",
    "override your",
    "act as if",
    "[system]",
];

/// Returns true if the content contains any known prompt injection pattern.
pub fn has_injection_pattern(content: &str) -> bool {
    let lower = content.to_lowercase();
    INJECTION_PATTERNS.iter().any(|p| lower.contains(p))
}

/// If the chunk contains a prompt injection pattern, add `warning:prompt-injection` tag.
/// Content is never modified — only tags are added. Idempotent.
pub fn flag_injection(chunk: &mut ImportChunk) {
    if has_injection_pattern(&chunk.content) {
        let tag = "warning:prompt-injection".to_string();
        if !chunk.tags.contains(&tag) {
            chunk.tags.push(tag);
        }
    }
}
