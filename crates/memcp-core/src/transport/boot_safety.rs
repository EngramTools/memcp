//! Phase 24.5 D-02 boot-safety helper.
//!
//! Refuse to start the HTTP API when it binds to a non-loopback interface and
//! no ingest API key is configured. The check lives here as a pure function so
//! the daemon entrypoint and integration tests exercise the exact same logic.
//!
//! Threat T-24.5-01 (unauthenticated ingest on exposed bind) is mitigated by
//! this helper being called BEFORE `axum::serve()`. See
//! `.planning/phases/24.5-universal-ingestion-api/24.5-RESEARCH.md` Pitfall 5
//! for why `starts_with("127.")` is NOT sufficient — we parse `IpAddr` and call
//! `is_loopback()` plus check the `"localhost"` string explicitly.

use std::net::IpAddr;

/// D-02: refuse to start when HTTP binds to a non-loopback interface without
/// a configured ingest API key. Returns `Ok(())` when safe, `Err(msg)` when
/// misconfigured. The caller (daemon entrypoint) prints `msg` to stderr and
/// exits 1.
///
/// Loopback is defined as:
///   * any IP where `IpAddr::is_loopback()` is true (`127.0.0.0/8`, `::1`), OR
///   * the literal string `"localhost"` (case-insensitive).
///
/// Non-loopback is everything else, including `0.0.0.0`, `::`, and any routable
/// address such as `192.168.1.5` or `10.0.0.1`.
///
/// The `bind` argument accepts both `"127.0.0.1:8080"` and `"127.0.0.1"`; the
/// port is stripped before parsing. IPv6 brackets (`[::1]:8080`) are also handled.
pub fn check_ingest_auth_safety(bind: &str, api_key: Option<&str>) -> Result<(), String> {
    // Fast path: literal "localhost" is always loopback.
    if bind.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }

    // Strip port if present. Handles:
    //   "127.0.0.1:8080" -> "127.0.0.1"
    //   "[::1]:8080"     -> "::1"
    //   "::1"            -> "::1" (no rsplit would strip the last colon incorrectly, but
    //                              leading '[' is absent so we keep bare form)
    //   "192.168.1.5"    -> "192.168.1.5"
    //
    // For IPv6 without brackets (e.g. bare "::1") `rsplit_once(':')` would chop the
    // final "::1" into "" + "1". Guard against that by only stripping when the
    // result parses cleanly, OR when brackets were used.
    let host = if let Some(stripped) = bind.strip_prefix('[') {
        // Bracketed IPv6, e.g. "[::1]:8080" or "[::1]"
        stripped
            .rsplit_once(']')
            .map(|(h, _)| h)
            .unwrap_or(stripped)
    } else if bind.parse::<IpAddr>().is_ok() {
        // Bare IP (including "::1") already parses — don't touch it.
        bind
    } else {
        // "host:port" form — strip the final ":<port>".
        bind.rsplit_once(':').map(|(h, _)| h).unwrap_or(bind)
    };

    match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() => Ok(()),
        _ => {
            if api_key.is_some() {
                Ok(())
            } else {
                Err(format!(
                    "HTTP server binds to non-loopback address '{}' but no [ingest] api_key is configured.\n\
                     Set MEMCP_INGEST__API_KEY or [ingest] api_key in memcp.toml, or bind to 127.0.0.1 / ::1 / localhost.",
                    bind
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_ipv4_no_key_ok() {
        assert!(check_ingest_auth_safety("127.0.0.1", None).is_ok());
        assert!(check_ingest_auth_safety("127.0.0.1:8080", None).is_ok());
        assert!(check_ingest_auth_safety("127.0.0.5", None).is_ok()); // whole /8
    }

    #[test]
    fn loopback_ipv6_no_key_ok() {
        assert!(check_ingest_auth_safety("::1", None).is_ok());
        assert!(check_ingest_auth_safety("[::1]:8080", None).is_ok());
        assert!(check_ingest_auth_safety("0:0:0:0:0:0:0:1", None).is_ok());
    }

    #[test]
    fn localhost_string_no_key_ok() {
        assert!(check_ingest_auth_safety("localhost", None).is_ok());
        assert!(check_ingest_auth_safety("LOCALHOST", None).is_ok()); // case-insensitive
    }

    #[test]
    fn non_loopback_no_key_err() {
        let err = check_ingest_auth_safety("0.0.0.0:8080", None).unwrap_err();
        assert!(err.contains("MEMCP_INGEST__API_KEY"));
        assert!(check_ingest_auth_safety("192.168.1.5", None).is_err());
        assert!(check_ingest_auth_safety("10.0.0.1:8080", None).is_err());
        assert!(check_ingest_auth_safety("203.0.113.42", None).is_err());
    }

    #[test]
    fn non_loopback_with_key_ok() {
        assert!(check_ingest_auth_safety("0.0.0.0:8080", Some("k")).is_ok());
        assert!(check_ingest_auth_safety("192.168.1.5", Some("key")).is_ok());
    }

    /// Regression guard for RESEARCH pitfall 5: naive `starts_with("127.")` would
    /// MISS IPv6 loopback and accept addresses that happen to start with "127."
    /// like "127.1.2.3" (still /8 loopback — our version catches this via IpAddr).
    #[test]
    fn regression_ipv6_loopback_not_missed_by_prefix_check() {
        // "::1" does NOT start with "127." — a prefix check would reject it.
        // is_loopback() correctly treats it as loopback.
        assert!(check_ingest_auth_safety("::1", None).is_ok());
    }
}
