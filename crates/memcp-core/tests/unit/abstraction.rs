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

/// Helper that mirrors the depth selection logic in transport/server.rs.
/// Returns a reference to the display content based on depth.
fn select_depth<'a>(
    depth: u8,
    content: &'a str,
    abstract_text: Option<&'a str>,
    overview_text: Option<&'a str>,
) -> &'a str {
    match depth {
        0 => abstract_text.unwrap_or(content),
        1 => overview_text.unwrap_or(content),
        _ => content,
    }
}

#[test]
fn test_depth_fallback_returns_content_when_abstract_null() {
    // TCL-05: depth=0 with no abstract_text returns content (graceful fallback)
    let result = select_depth(0, "full content", None, None);
    assert_eq!(result, "full content", "depth=0 with no abstract should fall back to content");
}

#[test]
fn test_depth_zero_returns_abstract() {
    // TCL-05: depth=0 returns abstract_text when present
    let result = select_depth(0, "full content", Some("short abstract"), None);
    assert_eq!(result, "short abstract", "depth=0 should return abstract_text");
    assert_ne!(result, "full content", "depth=0 should NOT return full content when abstract is present");
}

#[test]
fn test_depth_default_returns_full_content() {
    // TCL-05: depth=2 (default) returns full content regardless of abstract presence
    let result = select_depth(2, "full content", Some("abstract"), Some("overview"));
    assert_eq!(result, "full content", "depth=2 should always return full content");
}
