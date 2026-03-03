//! Deduplication for import pipeline.
//!
//! Uses SHA-256 of normalized content for cross-source dedup.
//! Normalization strips whitespace, lowercases, and removes markdown formatting.
//!
//! Two-level dedup:
//! 1. Batch-level: HashSet within the current import batch.
//! 2. Store-level: check existing memories in Postgres (windowed by IMPORT_DEDUP_WINDOW_DAYS).

use std::collections::HashSet;

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::warn;

/// Number of days to look back when checking existing memories for dedup.
/// Import is a one-time operation so a 30-day window is acceptable for MVP.
pub const IMPORT_DEDUP_WINDOW_DAYS: i32 = 30;

/// Normalize content for dedup comparison.
/// Strips leading/trailing whitespace, lowercases, removes markdown formatting characters.
pub fn normalize_content(content: &str) -> String {
    let mut s = content.trim().to_lowercase();

    // Remove common markdown formatting characters that don't affect meaning.
    // Headers: remove leading # characters and surrounding whitespace.
    s = s.lines()
        .map(|line| {
            let trimmed = line.trim_start_matches('#').trim();
            trimmed.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Remove bold/italic markers.
    s = s.replace("**", "").replace('*', "").replace("__", "").replace('_', "");

    // Remove blockquotes.
    s = s.lines()
        .map(|line| line.trim_start_matches('>').trim().to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // Remove code fence markers.
    s = s.replace("```", "").replace('`', "");

    // Remove list markers at line start.
    s = s.lines()
        .map(|line| {
            let trimmed = line.trim_start_matches("- ").trim_start_matches("* ").trim_start_matches("+ ");
            trimmed.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Collapse multiple whitespace/newlines into single space.
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in chars {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }

    result.trim().to_string()
}

/// Compute SHA-256 hash of normalized content.
/// Returns hex-encoded digest string.
pub fn normalized_hash(content: &str) -> String {
    let normalized = normalize_content(content);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Alias for normalized_hash — semantically clear name for import callers.
pub fn content_to_import_hash(content: &str) -> String {
    normalized_hash(content)
}

/// Check which of the provided normalized hashes already exist in the memories table.
///
/// NOTE: There is no normalized_hash column yet. For MVP, this queries content directly
/// from memories created in the last IMPORT_DEDUP_WINDOW_DAYS days, computes their
/// normalized hashes on the fly, and returns the set of matches.
///
/// This is acceptable for import (one-time operation). Future: add normalized_hash column.
pub async fn check_existing(pool: &PgPool, hashes: &[String]) -> anyhow::Result<HashSet<String>> {
    if hashes.is_empty() {
        return Ok(HashSet::new());
    }

    // Fetch existing content from memories within the dedup window.
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT content FROM memories
         WHERE content IS NOT NULL
           AND deleted_at IS NULL
           AND created_at >= NOW() - ($1 || ' days')::INTERVAL"
    )
    .bind(IMPORT_DEDUP_WINDOW_DAYS.to_string())
    .fetch_all(pool)
    .await
    .map_err(|e| {
        warn!("Dedup query failed, proceeding without store-level dedup: {}", e);
        e
    })?;

    // Build set of normalized hashes from existing memories.
    let existing_hashes: HashSet<String> = rows
        .into_iter()
        .map(|(content,)| normalized_hash(&content))
        .collect();

    // Return intersection — hashes from import batch that already exist in store.
    let input_set: HashSet<String> = hashes.iter().cloned().collect();
    Ok(existing_hashes.intersection(&input_set).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_strips_markdown() {
        let md = "## This is a Header\n**bold text** and _italic_\n- list item";
        let normalized = normalize_content(md);
        assert!(!normalized.contains('#'));
        assert!(!normalized.contains("**"));
        assert!(!normalized.contains('_'));
        assert!(!normalized.contains("- "));
    }

    #[test]
    fn test_normalize_lowercases() {
        let s = "Hello World UPPERCASE";
        let normalized = normalize_content(s);
        assert_eq!(normalized, "hello world uppercase");
    }

    #[test]
    fn test_normalized_hash_is_consistent() {
        let h1 = normalized_hash("Hello, World!");
        let h2 = normalized_hash("Hello, World!");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_normalized_hash_ignores_formatting() {
        // Same semantic content, different markdown formatting.
        let h1 = normalized_hash("User prefers dark mode");
        let h2 = normalized_hash("**User** prefers _dark mode_");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_normalized_hash_is_sha256_hex() {
        let h = normalized_hash("test content");
        // SHA-256 hex is always 64 characters.
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
