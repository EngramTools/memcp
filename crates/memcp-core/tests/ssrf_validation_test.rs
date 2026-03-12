//! SSRF prevention tests — validates that provider URLs are checked
//! for dangerous schemes and private IP ranges.
//!
//! SEC-05: Provider URLs must reject file://, private IPs, and AWS metadata.
//! SEC-06: HTTP localhost is allowed (Ollama default), HTTPS required for remote.

use memcp::validation::validate_provider_url;

// ── Dangerous schemes ───────────────────────────────────────────────────────

/// file:// URLs must be rejected (path traversal / local file read).
#[test]
fn test_reject_file_scheme() {
    let result = validate_provider_url("file:///etc/passwd", true);
    assert!(result.is_err(), "file:// URLs must be rejected");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("file://") || msg.contains("scheme"),
        "Error should mention the scheme, got: {}",
        msg
    );
}

/// ftp:// URLs must be rejected.
#[test]
fn test_reject_ftp_scheme() {
    let result = validate_provider_url("ftp://evil.com/data", true);
    assert!(result.is_err(), "ftp:// URLs must be rejected");
}

/// gopher:// URLs must be rejected.
#[test]
fn test_reject_gopher_scheme() {
    let result = validate_provider_url("gopher://evil.com", true);
    assert!(result.is_err(), "gopher:// URLs must be rejected");
}

// ── Private IP ranges ───────────────────────────────────────────────────────

/// AWS metadata endpoint (169.254.169.254) must be rejected.
#[test]
fn test_reject_aws_metadata() {
    let result = validate_provider_url("http://169.254.169.254/latest/meta-data/", true);
    assert!(
        result.is_err(),
        "AWS metadata IP 169.254.169.254 must be rejected"
    );
}

/// 10.x.x.x private range must be rejected.
#[test]
fn test_reject_private_10() {
    let result = validate_provider_url("http://10.0.0.1/api", true);
    assert!(result.is_err(), "10.x private IP must be rejected");
}

/// 172.16-31.x private range must be rejected.
#[test]
fn test_reject_private_172() {
    let result = validate_provider_url("http://172.16.0.1/api", true);
    assert!(result.is_err(), "172.16.x private IP must be rejected");
}

/// 192.168.x private range must be rejected.
#[test]
fn test_reject_private_192() {
    let result = validate_provider_url("http://192.168.1.1/api", true);
    assert!(result.is_err(), "192.168.x private IP must be rejected");
}

/// 169.254.x.x link-local must be rejected (not just the specific AWS metadata IP).
#[test]
fn test_reject_link_local() {
    let result = validate_provider_url("http://169.254.1.1/api", true);
    assert!(result.is_err(), "169.254.x link-local must be rejected");
}

// ── Allowed URLs ────────────────────────────────────────────────────────────

/// HTTP localhost is allowed when allow_localhost_http is true (Ollama default).
#[test]
fn test_allow_localhost_http() {
    let result = validate_provider_url("http://localhost:11434", true);
    assert!(
        result.is_ok(),
        "HTTP localhost must be allowed when allow_localhost_http=true, got: {:?}",
        result
    );
}

/// HTTP 127.0.0.1 is allowed when allow_localhost_http is true.
#[test]
fn test_allow_127_http() {
    let result = validate_provider_url("http://127.0.0.1:11434", true);
    assert!(
        result.is_ok(),
        "HTTP 127.0.0.1 must be allowed, got: {:?}",
        result
    );
}

/// HTTPS to any public host is allowed.
#[test]
fn test_allow_https_public() {
    let result = validate_provider_url("https://api.openai.com/v1", true);
    assert!(
        result.is_ok(),
        "HTTPS public URLs must be allowed, got: {:?}",
        result
    );
}

/// HTTPS to any host (even private IPs) is allowed — SSRF via HTTPS is less risky
/// and blocking would break legitimate internal services.
#[test]
fn test_allow_https_private() {
    let result = validate_provider_url("https://10.0.0.1/api", true);
    assert!(
        result.is_ok(),
        "HTTPS to private IPs should be allowed, got: {:?}",
        result
    );
}

/// HTTP localhost is rejected when allow_localhost_http is false.
#[test]
fn test_reject_localhost_http_when_disabled() {
    let result = validate_provider_url("http://localhost:11434", false);
    assert!(
        result.is_err(),
        "HTTP localhost must be rejected when allow_localhost_http=false"
    );
}

/// Invalid URL format is rejected.
#[test]
fn test_reject_invalid_url() {
    let result = validate_provider_url("not-a-url", true);
    assert!(result.is_err(), "Invalid URLs must be rejected");
}

/// Empty URL is rejected.
#[test]
fn test_reject_empty_url() {
    let result = validate_provider_url("", true);
    assert!(result.is_err(), "Empty URLs must be rejected");
}
