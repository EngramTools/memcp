//! Error sanitization tests — verifies that MemcpError never leaks
//! sensitive data (database URLs, credentials, file paths, raw sqlx errors)
//! in its Display output.
//!
//! SEC-04: Error messages must be safe for client consumption.

use memcp::errors::MemcpError;

// ── Storage error sanitization ──────────────────────────────────────────────

/// MemcpError created from a sqlx error must NOT contain the connection string.
#[test]
fn test_storage_error_hides_connection_string() {
    // Simulate a sqlx error that contains a full connection string.
    let raw_error = "error communicating with database: Connection refused (os error 111) \
                     postgres://user:s3cret@db.internal:5432/memcp_prod";
    let err = MemcpError::Storage(raw_error.to_string());
    let display = format!("{}", err);

    assert!(
        !display.contains("postgres://"),
        "Display output must not contain postgres:// URL, got: {}",
        display
    );
    assert!(
        !display.contains("s3cret"),
        "Display output must not contain credentials, got: {}",
        display
    );
    assert!(
        !display.contains("db.internal"),
        "Display output must not contain internal hostnames, got: {}",
        display
    );
}

/// MemcpError::Storage Display output should say "Database operation failed".
#[test]
fn test_storage_error_generic_message() {
    let err = MemcpError::Storage("anything".to_string());
    let display = format!("{}", err);

    assert!(
        display.contains("Database operation failed"),
        "Storage error Display must say 'Database operation failed', got: {}",
        display
    );
}

/// From<sqlx::Error> impl must sanitize before storing.
#[test]
fn test_from_sqlx_error_sanitizes() {
    // sqlx::Error doesn't have a simple public constructor, so we test
    // through the MemcpError::from_storage_error helper instead.
    let raw = "pool timed out while waiting for an open connection, \
               url=postgres://memcp:memcp@localhost:5433/memcp";
    let err = MemcpError::from_storage_error(raw);
    let display = format!("{}", err);

    assert!(
        !display.contains("postgres://"),
        "from_storage_error must sanitize URLs, got: {}",
        display
    );
    assert!(
        display.contains("Database operation failed"),
        "from_storage_error must produce generic message, got: {}",
        display
    );
}

/// Internal error variant also sanitizes paths.
#[test]
fn test_internal_error_hides_file_paths() {
    let err = MemcpError::Internal(
        "Failed to read /home/deploy/.config/memcp/secrets.toml: permission denied".to_string(),
    );
    let display = format!("{}", err);

    assert!(
        !display.contains("/home/deploy"),
        "Internal error must not contain file paths, got: {}",
        display
    );
}

/// Config error does not leak file paths.
#[test]
fn test_config_error_hides_paths() {
    let err = MemcpError::Config(
        "Failed to parse /etc/memcp/memcp.toml: invalid TOML".to_string(),
    );
    let display = format!("{}", err);

    assert!(
        !display.contains("/etc/memcp"),
        "Config error must not leak paths, got: {}",
        display
    );
}

/// Validation errors are safe (user-controlled content, no secrets).
#[test]
fn test_validation_error_passes_through() {
    let err = MemcpError::validation("content", "Content exceeds maximum size");
    let display = format!("{}", err);

    assert!(
        display.contains("Content exceeds maximum size"),
        "Validation errors should pass through unchanged, got: {}",
        display
    );
}

/// CapExceeded errors are safe (all values are numeric).
#[test]
fn test_cap_exceeded_error_passes_through() {
    let err = MemcpError::CapExceeded {
        cap: "memories".to_string(),
        limit: 1000,
        current: 1001,
    };
    let display = format!("{}", err);

    assert!(
        display.contains("memories") && display.contains("1000"),
        "CapExceeded errors should pass through unchanged, got: {}",
        display
    );
}

/// NotFound errors are safe (just IDs).
#[test]
fn test_not_found_error_passes_through() {
    let err = MemcpError::NotFound {
        id: "abc-123".to_string(),
    };
    let display = format!("{}", err);
    assert!(
        display.contains("abc-123"),
        "NotFound should contain the ID, got: {}",
        display
    );
}
