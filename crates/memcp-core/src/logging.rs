//! Tracing/logging initialization and content privacy utilities.
//!
//! Configures tracing-subscriber with env-filter. All output goes to stderr
//! (stdout reserved for JSON-RPC in MCP serve mode).
//!
//! Also provides `Redacted<T>` — a Display/Debug wrapper that redacts any value
//! to `[REDACTED]` so memory content never leaks into INFO-level logs.

/// Wrapper that redacts content in Display and Debug output.
///
/// Use for memory content fields that must not appear in INFO-level logs.
///
/// # Example
/// ```rust
/// use memcp::logging::Redacted;
/// let content = "my secret memory";
/// tracing::info!(content = %Redacted(&content), "storing memory");
/// // Logs: content=[REDACTED]
/// ```
pub struct Redacted<T>(pub T);

impl<T> std::fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl<T> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

/// Structured logging setup using tracing
///
/// CRITICAL: Writes to stderr ONLY (never stdout) to avoid corrupting JSON-RPC stream.
/// Auto-detects format: human-readable with ANSI colors when stderr is a terminal,
/// structured JSON when piped/redirected.

use std::io::IsTerminal;
use tracing_subscriber::{
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};
use crate::config::Config;

/// Initialize tracing subscriber with stderr-only output
///
/// Format auto-detection:
/// - Terminal: human-readable with ANSI colors
/// - Pipe/redirect: structured JSON
///
/// Log level from config.log_level (default: info)
/// RUST_LOG env var can override at runtime
pub fn init_logging(config: &Config) {
    // Build env filter from config, with RUST_LOG override
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    // Auto-detect format based on stderr terminal status
    let stderr_is_terminal = std::io::stderr().is_terminal();

    if stderr_is_terminal {
        // Human-readable format with ANSI colors for terminal
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(true)
            )
            .init();
    } else {
        // Structured JSON format for pipes/redirects
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .json()
            )
            .init();
    }

    // File-based logging is not yet implemented. When log_file is configured,
    // we warn and fall back to stderr. Implementation deferred to a future phase.
    if config.log_file.is_some() {
        tracing::warn!(
            "log_file configuration is not yet implemented, logging to stderr only"
        );
    }
}
