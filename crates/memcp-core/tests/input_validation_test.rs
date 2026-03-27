//! Integration tests for input validation — oversized content, excess tags, long tags, oversized query.
//!
//! Tests exercise the validation module directly (unit-level) since MCP/HTTP transport
//! integration tests require a running database. The validation functions are the
//! single enforcement point wired into all three transport layers.

use memcp::validation::{validate_content, validate_query, validate_tags, InputLimitsConfig};

#[test]
fn test_content_over_100kb_rejected() {
    let config = InputLimitsConfig::default();
    let oversized = "x".repeat(102_401); // 1 byte over 100KB
    let result = validate_content(&oversized, &config);
    assert!(result.is_err(), "Content over 100KB should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("content") || err.contains("Content"),
        "Error should mention content field: {}",
        err
    );
    assert!(
        err.contains("102400"),
        "Error should include the limit: {}",
        err
    );
}

#[test]
fn test_content_exactly_100kb_accepted() {
    let config = InputLimitsConfig::default();
    let exact = "x".repeat(102_400); // Exactly 100KB
    let result = validate_content(&exact, &config);
    assert!(
        result.is_ok(),
        "Content exactly at limit should be accepted"
    );
}

#[test]
fn test_too_many_tags_rejected() {
    let config = InputLimitsConfig::default();
    let tags: Vec<String> = (0..33).map(|i| format!("tag{}", i)).collect();
    let result = validate_tags(&tags, &config);
    assert!(result.is_err(), "More than 32 tags should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("tags") || err.contains("Too many"),
        "Error should mention tags: {}",
        err
    );
    assert!(
        err.contains("32"),
        "Error should include the limit: {}",
        err
    );
}

#[test]
fn test_exactly_32_tags_accepted() {
    let config = InputLimitsConfig::default();
    let tags: Vec<String> = (0..32).map(|i| format!("tag{}", i)).collect();
    let result = validate_tags(&tags, &config);
    assert!(result.is_ok(), "Exactly 32 tags should be accepted");
}

#[test]
fn test_long_tag_rejected() {
    let config = InputLimitsConfig::default();
    let long_tag = "a".repeat(257);
    let tags = vec![long_tag];
    let result = validate_tags(&tags, &config);
    assert!(result.is_err(), "Tag over 256 chars should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("256"),
        "Error should include the limit: {}",
        err
    );
}

#[test]
fn test_tag_exactly_256_chars_accepted() {
    let config = InputLimitsConfig::default();
    let tag = "a".repeat(256);
    let tags = vec![tag];
    let result = validate_tags(&tags, &config);
    assert!(
        result.is_ok(),
        "Tag exactly at 256 chars should be accepted"
    );
}

#[test]
fn test_query_over_10kb_rejected() {
    let config = InputLimitsConfig::default();
    let oversized = "q".repeat(10_241);
    let result = validate_query(&oversized, &config);
    assert!(result.is_err(), "Query over 10KB should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("query") || err.contains("Query"),
        "Error should mention query field: {}",
        err
    );
    assert!(
        err.contains("10240"),
        "Error should include the limit: {}",
        err
    );
}

#[test]
fn test_query_exactly_10kb_accepted() {
    let config = InputLimitsConfig::default();
    let exact = "q".repeat(10_240);
    let result = validate_query(&exact, &config);
    assert!(result.is_ok(), "Query exactly at limit should be accepted");
}

#[test]
fn test_validation_errors_include_field_and_limit() {
    let config = InputLimitsConfig::default();

    // Content error mentions field and limit
    let err = validate_content(&"x".repeat(200_000), &config).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("102400"),
        "Content error missing limit: {}",
        msg
    );

    // Tag count error mentions field and limit
    let tags: Vec<String> = (0..50).map(|i| format!("t{}", i)).collect();
    let err = validate_tags(&tags, &config).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("32"), "Tag count error missing limit: {}", msg);

    // Query error mentions field and limit
    let err = validate_query(&"q".repeat(20_000), &config).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("10240"), "Query error missing limit: {}", msg);
}

#[test]
fn test_custom_config_limits() {
    let config = InputLimitsConfig {
        max_content_bytes: 50,
        max_tag_count: 2,
        max_tag_length: 10,
        max_query_length: 20,
        max_batch_size: 5,
        allow_localhost_http: true,
    };

    // Content over custom limit
    assert!(validate_content(&"x".repeat(51), &config).is_err());
    assert!(validate_content(&"x".repeat(50), &config).is_ok());

    // Tags over custom limit
    let tags: Vec<String> = (0..3).map(|i| format!("t{}", i)).collect();
    assert!(validate_tags(&tags, &config).is_err());

    // Tag length over custom limit
    let tags = vec!["a".repeat(11)];
    assert!(validate_tags(&tags, &config).is_err());

    // Query over custom limit
    assert!(validate_query(&"q".repeat(21), &config).is_err());
    assert!(validate_query(&"q".repeat(20), &config).is_ok());
}
