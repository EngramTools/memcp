//! Centralized input validation for all transport entry points.
//!
//! Provides configurable limits for content size, tag count/length, query length,
//! and batch size. All validators return `Result<(), MemcpError>` using the
//! existing `MemcpError::validation()` constructor.

use crate::errors::MemcpError;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

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

    /// Allow HTTP (non-TLS) connections to localhost/127.0.0.1 for local providers.
    /// Default: true — required for Ollama which runs on http://localhost:11434.
    /// SEC-06: Set to false to require HTTPS for all provider connections.
    #[serde(default = "default_allow_localhost_http")]
    pub allow_localhost_http: bool,
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
fn default_allow_localhost_http() -> bool {
    true
}

impl Default for InputLimitsConfig {
    fn default() -> Self {
        Self {
            max_content_bytes: default_max_content_bytes(),
            max_tag_count: default_max_tag_count(),
            max_tag_length: default_max_tag_length(),
            max_query_length: default_max_query_length(),
            max_batch_size: default_max_batch_size(),
            allow_localhost_http: default_allow_localhost_http(),
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

// ── SSRF prevention ─────────────────────────────────────────────────────────

/// Validate a provider URL against SSRF attacks.
///
/// SEC-06: Prevents server-side request forgery by restricting URL schemes and targets.
///
/// **Allowed:**
/// - HTTPS to any host (including private IPs — needed for internal services)
/// - HTTP to localhost/127.0.0.1/::1 when `allow_localhost_http` is true (Ollama default)
///
/// **Rejected:**
/// - `file://`, `ftp://`, `gopher://` and all non-HTTP(S) schemes
/// - HTTP to private IP ranges: 10.x, 172.16-31.x, 192.168.x, 169.254.x (link-local/AWS metadata)
/// - HTTP to any non-localhost host when `allow_localhost_http` is false
pub fn validate_provider_url(url_str: &str, allow_localhost_http: bool) -> Result<(), MemcpError> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| MemcpError::validation("url", &format!("Invalid URL '{}': {}", url_str, e)))?;

    let scheme = parsed.scheme();

    // Only allow http and https schemes
    if scheme != "http" && scheme != "https" {
        return Err(MemcpError::validation(
            "url",
            &format!(
                "Unsupported URL scheme '{}://'. Only http:// and https:// are allowed.",
                scheme
            ),
        ));
    }

    // HTTPS is always allowed (even to private IPs — needed for internal services)
    if scheme == "https" {
        return Ok(());
    }

    // HTTP: check host restrictions
    let host = parsed.host_str().unwrap_or("");

    // Check if host is localhost
    let is_localhost =
        host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]";

    if is_localhost {
        if allow_localhost_http {
            return Ok(());
        } else {
            return Err(MemcpError::validation(
                "url",
                "HTTP to localhost is disabled. Use HTTPS or enable allow_localhost_http.",
            ));
        }
    }

    // HTTP to non-localhost: check for private/reserved IP ranges
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(&ip) {
            return Err(MemcpError::validation(
                "url",
                &format!(
                    "HTTP to private/reserved IP {} is not allowed. Use HTTPS for internal services.",
                    ip
                ),
            ));
        }
    }

    // HTTP to public non-localhost host — reject (require HTTPS for remote)
    Err(MemcpError::validation(
        "url",
        &format!(
            "HTTP to remote host '{}' is not allowed. Use HTTPS for remote providers.",
            host
        ),
    ))
}

/// Check if an IP address is in a private or reserved range.
///
/// Private ranges: 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
/// Link-local: 169.254.0.0/16 (includes AWS metadata at 169.254.169.254)
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 10.0.0.0/8
            octets[0] == 10
            // 172.16.0.0/12
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            // 192.168.0.0/16
            || (octets[0] == 192 && octets[1] == 168)
            // 169.254.0.0/16 (link-local, includes AWS metadata)
            || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(_v6) => {
            // For now, allow IPv6 — SSRF via IPv6 is uncommon in this context.
            // Could add fc00::/7 (unique local) and fe80::/10 (link-local) checks later.
            false
        }
    }
}
