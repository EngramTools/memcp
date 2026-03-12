//! Security helpers for the import pipeline.
//!
//! Provides:
//! - Prompt injection detection and flagging
//! - ZIP bomb protection constants (used by ZIP readers)
//! - Path traversal protection for ZIP entries (SEC-05)
//! - Per-file size limits for ZIP extraction (SEC-05)

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

/// Maximum size for a single file within a ZIP archive (50MB).
/// SEC-05: Prevents resource exhaustion from oversized individual entries.
pub const MAX_SINGLE_FILE_SIZE: u64 = 50 * 1024 * 1024;

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

/// Check if a ZIP entry name is safe (no path traversal).
///
/// SEC-05: Rejects entries with:
/// - `..` path components (directory traversal)
/// - Absolute paths starting with `/` or `\`
/// - Backslash traversal (`..\\`)
///
/// Returns `true` if the entry name is safe, `false` if it should be skipped.
pub fn is_safe_zip_entry_name(name: &str) -> bool {
    // Reject empty names
    if name.is_empty() {
        return false;
    }

    // Reject absolute paths (Unix or Windows)
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }

    // Normalize backslashes to forward slashes for uniform checking
    let normalized = name.replace('\\', "/");

    // Reject any path component that is ".."
    for component in normalized.split('/') {
        if component == ".." {
            return false;
        }
    }

    true
}
