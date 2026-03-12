// Tests for AbstractionProvider and depth fallback logic

use memcp::config::AbstractionConfig;
use memcp::embedding::build_embedding_text;
use memcp::pipeline::abstraction::create_abstraction_provider;

#[test]
fn test_create_abstraction_provider_disabled() {
    // TCL-01: create_abstraction_provider returns None when disabled
    let config = AbstractionConfig::default(); // enabled: false
    let result = create_abstraction_provider(&config).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_create_abstraction_provider_ollama() {
    // TCL-01: create_abstraction_provider returns Some for ollama
    let mut config = AbstractionConfig::default();
    config.enabled = true;
    let result = create_abstraction_provider(&config).unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().model_name(), "llama3.2:3b");
}

#[test]
fn test_create_abstraction_provider_openai_missing_key() {
    // TCL-01: openai without key returns error
    let mut config = AbstractionConfig::default();
    config.enabled = true;
    config.provider = "openai".to_string();
    let result = create_abstraction_provider(&config);
    assert!(result.is_err());
}

#[test]
fn test_embed_uses_abstract_text() {
    // TCL-02: build_embedding_text prefers abstract_text over content
    let result = build_embedding_text("full content", Some("abstract"), &None);
    assert!(result.contains("abstract"), "Expected result to contain 'abstract', got: {result}");
    assert!(!result.contains("full content"), "Expected result NOT to contain 'full content', got: {result}");
}

#[test]
fn test_embed_falls_back_to_content() {
    // TCL-02: build_embedding_text uses content when abstract_text is None
    let result = build_embedding_text("full content", None, &None);
    assert!(result.contains("full content"), "Expected result to contain 'full content', got: {result}");
}

#[test]
#[ignore = "requires Plan 03: depth parameter"]
fn test_depth_fallback_returns_content_when_abstract_null() {
    // TCL-05: depth=0 with no abstract_text returns content
    todo!("Plan 03 fills this in")
}

#[test]
#[ignore = "requires Plan 03: depth parameter"]
fn test_depth_zero_returns_abstract() {
    // TCL-05: depth=0 returns abstract_text when present
    todo!("Plan 03 fills this in")
}

#[test]
#[ignore = "requires Plan 03: depth parameter"]
fn test_depth_default_returns_full_content() {
    // TCL-05: depth=2 (default) returns full content
    todo!("Plan 03 fills this in")
}
