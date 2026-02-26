//! Summarization config and factory tests.
//!
//! These tests focus on the SummarizationConfig defaults and the
//! create_summarization_provider factory. No DB or external LLM calls needed
//! — uses #[test] or #[tokio::test].

use memcp::summarization::create_summarization_provider;
use memcp::config::SummarizationConfig;

// ---------------------------------------------------------------------------
// Test 1: SummarizationConfig defaults to disabled
// ---------------------------------------------------------------------------

#[test]
fn test_summarization_config_defaults() {
    let config = SummarizationConfig::default();
    assert!(
        !config.enabled,
        "SummarizationConfig should be disabled by default"
    );
}

// ---------------------------------------------------------------------------
// Test 2: create_summarization_provider returns None when disabled
// ---------------------------------------------------------------------------

#[test]
fn test_create_summarization_provider_disabled() {
    let config = SummarizationConfig {
        enabled: false,
        ..Default::default()
    };
    let result = create_summarization_provider(&config);
    assert!(result.is_ok(), "disabled config should not error");
    assert!(
        result.unwrap().is_none(),
        "disabled config should return None"
    );
}

// ---------------------------------------------------------------------------
// Test 3: create_summarization_provider with invalid openai config errors
// ---------------------------------------------------------------------------

#[test]
fn test_create_summarization_provider_openai_without_key_errors() {
    let config = SummarizationConfig {
        enabled: true,
        provider: "openai".to_string(),
        openai_api_key: None, // Missing — required for openai provider
        ..Default::default()
    };
    let result = create_summarization_provider(&config);
    assert!(
        result.is_err(),
        "openai provider without API key should return Err"
    );
}

// ---------------------------------------------------------------------------
// Test 4: create_summarization_provider with ollama returns Some (no external call)
// ---------------------------------------------------------------------------

#[test]
fn test_create_summarization_provider_ollama_returns_some() {
    let config = SummarizationConfig {
        enabled: true,
        provider: "ollama".to_string(),
        ..Default::default()
    };
    let result = create_summarization_provider(&config);
    assert!(result.is_ok(), "ollama config should not error at construction time");
    assert!(
        result.unwrap().is_some(),
        "enabled ollama config should return Some provider"
    );
}
