use memcp::extraction::build_extraction_prompt;

#[test]
fn test_build_extraction_prompt_includes_content() {
    let prompt = build_extraction_prompt("hello world");
    assert!(
        prompt.contains("hello world"),
        "prompt should contain the input content"
    );
}

#[test]
fn test_build_extraction_prompt_includes_instructions() {
    let prompt = build_extraction_prompt("some text");
    assert!(
        prompt.contains("entities") || prompt.contains("Entities"),
        "prompt should mention entities"
    );
    assert!(
        prompt.contains("facts") || prompt.contains("Facts"),
        "prompt should mention facts"
    );
}

#[test]
fn test_build_extraction_prompt_empty_content() {
    let prompt = build_extraction_prompt("");
    assert!(!prompt.is_empty(), "prompt should still be a valid string");
    // Should contain the instruction text even with empty content
    assert!(prompt.contains("Extract"));
}
