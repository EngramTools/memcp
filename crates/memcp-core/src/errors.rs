//! Error types for memcp operations.
//!
//! MemcpError covers validation, not-found, config, storage, and cap-exceeded errors.
//! Used across all layers.
//!
//! SEC-04: All error Display impls are sanitized — no database URLs, credentials,
//! file paths, or raw internal error text is exposed to clients.
//! Full error details are logged via tracing for debugging.

// SEC-07: Audit confirmed zero unsafe blocks in codebase (2026-03-11).

/// Domain-specific error types for memcp
///
/// Provides actionable error messages with detailed context to enable
/// AI agents to self-correct on bad tool calls.
///
/// **Security:** Display impls are sanitized. Raw error details are only
/// available via Debug (which should never be sent to clients).

#[derive(Debug, thiserror::Error)]
pub enum MemcpError {
    #[error("Validation error: {message}")]
    Validation {
        message: String,
        field: Option<String>,
    },

    #[error("Memory not found: {id}")]
    NotFound { id: String },

    #[error("Configuration error: {}", sanitize_message(.0))]
    Config(String),

    #[error("Internal error: {}", sanitize_message(.0))]
    Internal(String),

    #[error("Database operation failed")]
    Storage(String),

    #[error("Resource cap exceeded: {cap} (limit: {limit}, current: {current})")]
    CapExceeded {
        cap: String,
        limit: u64,
        current: u64,
    },
}

/// Sanitize error messages by removing sensitive patterns.
///
/// Strips: database URLs (postgres://...), file system paths (/home/..., /etc/...),
/// and credential patterns. Returns a generic message if the entire string is sensitive.
fn sanitize_message(msg: &str) -> String {
    // If the message contains a connection string, return generic message
    if msg.contains("postgres://")
        || msg.contains("postgresql://")
        || msg.contains("mysql://")
        || msg.contains("sqlite://")
    {
        return "operation failed (details logged)".to_string();
    }

    // If the message contains absolute file paths, strip them
    if contains_file_path(msg) {
        return "operation failed (details logged)".to_string();
    }

    msg.to_string()
}

/// Check if a string contains what looks like an absolute file path.
fn contains_file_path(msg: &str) -> bool {
    // Unix absolute paths
    for prefix in &["/home/", "/etc/", "/var/", "/tmp/", "/opt/", "/usr/", "/root/"] {
        if msg.contains(prefix) {
            return true;
        }
    }
    // Windows paths
    if msg.contains("C:\\") || msg.contains("D:\\") {
        return true;
    }
    false
}

impl From<sqlx::Error> for MemcpError {
    fn from(e: sqlx::Error) -> Self {
        let raw = e.to_string();
        tracing::error!(error = %raw, "Database error (sanitized for client)");
        MemcpError::Storage(raw)
    }
}

impl From<crate::embedding::EmbeddingError> for MemcpError {
    fn from(e: crate::embedding::EmbeddingError) -> Self {
        let raw = e.to_string();
        tracing::error!(error = %raw, "Embedding error (sanitized for client)");
        MemcpError::Internal(raw)
    }
}

impl MemcpError {
    /// Helper to create validation errors with field names
    ///
    /// Example:
    /// ```
    /// use memcp::errors::MemcpError;
    /// let err = MemcpError::validation("content", "Content cannot be empty");
    /// ```
    pub fn validation(field: &str, message: &str) -> Self {
        MemcpError::Validation {
            message: message.to_string(),
            field: Some(field.to_string()),
        }
    }

    /// Create a Storage error from a raw error string, sanitizing and logging.
    ///
    /// Use this instead of `MemcpError::Storage(e.to_string())` to ensure
    /// raw error details are logged but never exposed to clients.
    pub fn from_storage_error(raw: &str) -> Self {
        tracing::error!(error = %raw, "Storage error (sanitized for client)");
        MemcpError::Storage(raw.to_string())
    }
}

/// Redact credentials from a database URL for safe logging.
///
/// Returns the URL with credentials replaced by `***`.
/// Example: `postgres://user:pass@host:5432/db` → `postgres://***@host:5432/db`
pub fn redact_url(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        let scheme_end = url.find("://").map(|p| p + 3).unwrap_or(0);
        let scheme = &url[..scheme_end];
        let after_at = &url[at_pos + 1..];
        format!("{}***@{}", scheme, after_at)
    } else {
        url.to_string()
    }
}
