//! Unit tests for Shannon entropy calculation.

use memcp::pipeline::redaction::entropy::shannon_entropy;

#[test]
fn test_empty_string_entropy() {
    assert_eq!(shannon_entropy(""), 0.0);
}

#[test]
fn test_low_entropy_repetitive() {
    let ent = shannon_entropy("aaaa");
    assert!(ent < 1.0, "repetitive string should have entropy < 1.0, got {ent}");
}

#[test]
fn test_high_entropy_real_key() {
    let ent = shannon_entropy("sk-ant-api03-AbCdEf1234567890xYz");
    assert!(ent > 3.5, "real API key should have entropy > 3.5, got {ent}");
}

#[test]
fn test_low_entropy_placeholder() {
    let ent = shannon_entropy("your-key-here");
    assert!(ent < 3.5, "placeholder should have entropy < 3.5, got {ent}");
}

#[test]
fn test_single_char() {
    assert_eq!(shannon_entropy("a"), 0.0);
}

#[test]
fn test_two_distinct_chars() {
    let ent = shannon_entropy("ab");
    assert!((ent - 1.0).abs() < 0.01, "two distinct chars should have entropy ~1.0, got {ent}");
}
