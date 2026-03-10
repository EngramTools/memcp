use memcp::auto_store::filter::{CategoryFilter, FilterStrategy, HeuristicFilter, NoFilter};
use memcp::auto_store::parser::ParsedEntry;
use std::collections::HashMap;

fn make_entry(content: &str) -> ParsedEntry {
    ParsedEntry {
        content: content.to_string(),
        timestamp: None,
        source: "test".to_string(),
        actor: None,
        session_id: None,
        project: None,
        metadata: HashMap::new(),
    }
}

#[tokio::test]
async fn test_no_filter_always_stores() {
    let filter = NoFilter;
    assert!(filter.should_store(&make_entry("anything")).await.unwrap());
    assert!(filter.should_store(&make_entry("")).await.unwrap());
}

#[tokio::test]
async fn test_heuristic_filter_triggers() {
    let filter = HeuristicFilter;

    // Keyword triggers
    assert!(filter
        .should_store(&make_entry("Always use pnpm"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("never commit without tests"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("We prefer TypeScript"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("Remember to run lint"))
        .await
        .unwrap());

    // Short non-triggering content
    assert!(!filter.should_store(&make_entry("ok")).await.unwrap());
    assert!(!filter.should_store(&make_entry("thanks")).await.unwrap());
}

#[tokio::test]
async fn test_heuristic_filter_long_declarative() {
    let filter = HeuristicFilter;
    let long_text = "The project uses a microservices architecture with gRPC for inter-service communication. Each service has its own database.";
    assert!(filter.should_store(&make_entry(long_text)).await.unwrap());
}

fn make_category_filter(extra_patterns: Vec<String>) -> CategoryFilter {
    let config = memcp::config::CategoryFilterConfig {
        enabled: true,
        block_tool_narration: true,
        tool_narration_patterns: extra_patterns,
        category_actions: std::collections::HashMap::new(),
        llm_provider: None,
        llm_model: None,
    };
    CategoryFilter::new(&config, None)
}

#[tokio::test]
async fn test_category_filter_blocks_narration() {
    let filter = make_category_filter(vec![]);

    // Built-in narration patterns must be blocked
    assert!(!filter
        .should_store(&make_entry("Let me read the file"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Let me check the code"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Now I'll edit the code"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Now I will run the test"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Reading the file src/main.rs"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Running command ls -la"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("I'll start by looking at the code"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("I will begin by reading the directory"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Here's what I found"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Here is the output"))
        .await
        .unwrap());

    // Verify filtered_count is tracking correctly
    assert!(filter.filtered_count() > 0);
}

#[tokio::test]
async fn test_category_filter_passes_decisions() {
    let filter = make_category_filter(vec![]);

    // Decisions, preferences, errors, and architecture notes must pass through
    assert!(filter
        .should_store(&make_entry("Always use pnpm for this project"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("The architecture uses microservices"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("Error: connection refused"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("We decided to use PostgreSQL for the database"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("User prefers TypeScript over JavaScript"))
        .await
        .unwrap());
    assert!(filter
        .should_store(&make_entry("Important: never commit secrets to git"))
        .await
        .unwrap());

    // No count incremented for pass-throughs
    assert_eq!(filter.filtered_count(), 0);
}

#[tokio::test]
async fn test_category_filter_custom_patterns() {
    let custom = vec![
        r"(?i)^analyzing ".to_string(),
        r"(?i)^processing ".to_string(),
    ];
    let filter = make_category_filter(custom);

    // Custom patterns should be applied
    assert!(!filter
        .should_store(&make_entry("Analyzing the results now"))
        .await
        .unwrap());
    assert!(!filter
        .should_store(&make_entry("Processing the request"))
        .await
        .unwrap());

    // Built-in patterns still work
    assert!(!filter
        .should_store(&make_entry("Let me read the file"))
        .await
        .unwrap());

    // Non-matching content passes through
    assert!(filter
        .should_store(&make_entry("We decided to use Redis for caching"))
        .await
        .unwrap());
}

#[tokio::test]
async fn test_category_filter_bad_pattern_skipped() {
    // An invalid regex pattern must not crash the filter — fail-open
    let bad_patterns = vec![
        r"[invalid regex (missing close bracket".to_string(),
        r"(?i)^valid pattern ".to_string(),
    ];
    // This must not panic
    let filter = make_category_filter(bad_patterns);

    // The valid custom pattern should still work
    assert!(!filter
        .should_store(&make_entry("valid pattern match"))
        .await
        .unwrap());

    // Built-in patterns work too (construction succeeded despite bad pattern)
    assert!(!filter
        .should_store(&make_entry("Let me check the code"))
        .await
        .unwrap());

    // Normal content passes through
    assert!(filter
        .should_store(&make_entry("We prefer Rust for performance"))
        .await
        .unwrap());
}
