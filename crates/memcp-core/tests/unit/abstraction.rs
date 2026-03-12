// Tests for AbstractionProvider and depth fallback logic
// These are placeholders — implementation fills them in as plans 01-03 execute.

#[test]
#[ignore = "requires Plan 01: AbstractionConfig + provider"]
fn test_create_abstraction_provider_disabled() {
    // TCL-01: create_abstraction_provider returns None when disabled
    todo!("Plan 01 fills this in")
}

#[test]
#[ignore = "requires Plan 01: AbstractionConfig + provider"]
fn test_create_abstraction_provider_ollama() {
    // TCL-01: create_abstraction_provider returns Some for ollama
    todo!("Plan 01 fills this in")
}

#[test]
#[ignore = "requires Plan 01: AbstractionConfig + provider"]
fn test_create_abstraction_provider_openai_missing_key() {
    // TCL-01: openai without key returns error
    todo!("Plan 01 fills this in")
}

#[test]
#[ignore = "requires Plan 02: embedding pipeline"]
fn test_embed_uses_abstract_text() {
    // TCL-02: build_embedding_text prefers abstract_text over content
    todo!("Plan 02 fills this in")
}

#[test]
#[ignore = "requires Plan 02: embedding pipeline"]
fn test_embed_falls_back_to_content() {
    // TCL-02: build_embedding_text uses content when abstract_text is None
    todo!("Plan 02 fills this in")
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
