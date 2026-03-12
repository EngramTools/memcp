//! Centralized input validation for all transport entry points.
//!
//! Provides configurable limits for content size, tag count/length, query length,
//! and batch size. All validators return `Result<(), MemcpError>` using the
//! existing `MemcpError::validation()` constructor.

use crate::errors::MemcpError;
use serde::{Deserialize, Serialize};

/// Configurable input size limits.
///
/// All defaults are safe for single-instance deployments. Engram.host may
/// tighten these per-tier via `[input_limits]` in memcp.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputLimitsConfig {
    /// Maximum content size in bytes (default: 102400 = 100KB).
    #[serde(default = "default_max_content_bytes")]
    pub max_content_bytes: usize,

    /// Maximum number of tags per memory (default: 32).
    #[serde(default = "default_max_tag_count")]
    pub max_tag_count: usize,

    /// Maximum length of a single tag in characters (default: 256).
    #[serde(default = "default_max_tag_length")]
    pub max_tag_length: usize,

    /// Maximum query string length in bytes (default: 10240 = 10KB).
    #[serde(default = "default_max_query_length")]
    pub max_query_length: usize,

    /// Maximum batch size for bulk operations (default: 100).
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
}

fn default_max_content_bytes() -> usize {
    102_400
}
fn default_max_tag_count() -> usize {
    32
}
fn default_max_tag_length() -> usize {
    256
}
fn default_max_query_length() -> usize {
    10_240
}
fn default_max_batch_size() -> usize {
    100
}

impl Default for InputLimitsConfig {
    fn default() -> Self {
        Self {
            max_content_bytes: default_max_content_bytes(),
            max_tag_count: default_max_tag_count(),
            max_tag_length: default_max_tag_length(),
            max_query_length: default_max_query_length(),
            max_batch_size: default_max_batch_size(),
        }
    }
}

/// Validate content size against configured limit.
///
/// Returns `MemcpError::Validation` with field name and human-readable message
/// including the limit and actual size.
pub fn validate_content(content: &str, config: &InputLimitsConfig) -> Result<(), MemcpError> {
    if content.len() > config.max_content_bytes {
        return Err(MemcpError::validation(
            "content",
            &format!(
                "Content exceeds maximum size of {} bytes (got {} bytes)",
                config.max_content_bytes,
                content.len()
            ),
        ));
    }
    Ok(())
}

/// Validate tags: count and individual tag length.
///
/// Checks both the total number of tags and each tag's character length.
pub fn validate_tags(tags: &[String], config: &InputLimitsConfig) -> Result<(), MemcpError> {
    if tags.len() > config.max_tag_count {
        return Err(MemcpError::validation(
            "tags",
            &format!(
                "Too many tags: maximum is {} (got {})",
                config.max_tag_count,
                tags.len()
            ),
        ));
    }
    for (i, tag) in tags.iter().enumerate() {
        if tag.len() > config.max_tag_length {
            return Err(MemcpError::validation(
                "tags",
                &format!(
                    "Tag at index {} exceeds maximum length of {} characters (got {})",
                    i,
                    config.max_tag_length,
                    tag.len()
                ),
            ));
        }
    }
    Ok(())
}

/// Validate query string length.
pub fn validate_query(query: &str, config: &InputLimitsConfig) -> Result<(), MemcpError> {
    if query.len() > config.max_query_length {
        return Err(MemcpError::validation(
            "query",
            &format!(
                "Query exceeds maximum length of {} bytes (got {} bytes)",
                config.max_query_length,
                query.len()
            ),
        ));
    }
    Ok(())
}

/// Validate batch size for bulk operations.
pub fn validate_batch_size(size: usize, config: &InputLimitsConfig) -> Result<(), MemcpError> {
    if size > config.max_batch_size {
        return Err(MemcpError::validation(
            "batch_size",
            &format!(
                "Batch size exceeds maximum of {} (got {})",
                config.max_batch_size, size
            ),
        ));
    }
    Ok(())
}
