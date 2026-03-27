use memcp::consolidation::{build_synthesis_prompt, concatenate_memories};

#[test]
fn test_build_synthesis_prompt_includes_all_memories() {
    let contents = vec!["first memory", "second memory", "third memory"];
    let prompt = build_synthesis_prompt(&contents);
    assert!(prompt.contains("Memory 1:"), "should label Memory 1");
    assert!(prompt.contains("Memory 2:"), "should label Memory 2");
    assert!(prompt.contains("Memory 3:"), "should label Memory 3");
    assert!(prompt.contains("first memory"), "should include content 1");
    assert!(prompt.contains("second memory"), "should include content 2");
    assert!(prompt.contains("third memory"), "should include content 3");
}

#[test]
fn test_build_synthesis_prompt_single_memory() {
    let contents = vec!["only memory"];
    let prompt = build_synthesis_prompt(&contents);
    assert!(prompt.contains("Memory 1:"));
    assert!(prompt.contains("only memory"));
    assert!(!prompt.contains("Memory 2:"));
}

#[test]
fn test_concatenate_memories_multiple() {
    let contents = vec!["alpha", "beta"];
    let result = concatenate_memories(&contents);
    assert!(result.contains("---"), "should contain separator");
    assert!(result.contains("alpha"));
    assert!(result.contains("beta"));
}

#[test]
fn test_concatenate_memories_single() {
    let contents = vec!["solo"];
    let result = concatenate_memories(&contents);
    assert!(result.contains("solo"));
    assert!(
        !result.contains("---"),
        "single item should have no separator"
    );
}
