//! Auto-store sidecar E2E tests.
//!
//! Tests the AutoStoreWorker parser pipeline directly (no daemon subprocess).
//! Uses the LogParser implementations to process simulated JSONL conversation files.
//!
//! Test 1: Parser ingests multi-turn JSONL conversation and returns entries.
//! Test 2: Parser skips empty/whitespace messages.

use memcp::auto_store::parser::{ClaudeCodeParser, LogParser};
use std::path::Path;

// ---------------------------------------------------------------------------
// Test 1: Parser ingests simulated Claude Code JSONL conversation
// ---------------------------------------------------------------------------

#[test]
fn test_auto_store_ingests_jsonl() {
    // Simulate a Claude Code JSONL conversation with 4 turns
    let jsonl_lines = vec![
        // Human turn 1
        r#"{"type":"human","message":{"role":"user","content":"What database should I use?"},"timestamp":"2026-01-01T10:00:00Z","sessionId":"session-abc","cwd":"/Users/dev/project"}"#,
        // Assistant turn 1
        r#"{"type":"assistant","message":{"role":"assistant","content":"For your use case, PostgreSQL with pgvector is ideal for semantic search."},"timestamp":"2026-01-01T10:00:05Z","sessionId":"session-abc","cwd":"/Users/dev/project"}"#,
        // Human turn 2
        r#"{"type":"human","message":{"role":"user","content":"Set up dark mode preference"},"timestamp":"2026-01-01T10:01:00Z","sessionId":"session-abc","cwd":"/Users/dev/project"}"#,
        // Assistant turn 2
        r#"{"type":"assistant","message":{"role":"assistant","content":"I'll remember that you prefer dark mode in all editors and terminals."},"timestamp":"2026-01-01T10:01:05Z","sessionId":"session-abc","cwd":"/Users/dev/project"}"#,
    ];

    let parser = ClaudeCodeParser;
    let fake_path = Path::new("/fake/test.jsonl");

    let mut entries = Vec::new();
    for line in &jsonl_lines {
        if let Some(entry) = parser.parse_line(line, fake_path) {
            entries.push(entry);
        }
    }

    // All 4 lines are user/assistant messages — all should parse
    assert_eq!(
        entries.len(),
        4,
        "parser should return 4 entries (2 user + 2 assistant)"
    );

    // Verify content is extracted correctly
    let contents: Vec<&str> = entries.iter().map(|e| e.content.as_str()).collect();
    assert!(
        contents.contains(&"What database should I use?"),
        "user question should be present: {:?}",
        contents
    );
    assert!(
        contents
            .iter()
            .any(|c| c.contains("PostgreSQL with pgvector")),
        "assistant PostgreSQL response should be present: {:?}",
        contents
    );
    assert!(
        contents.iter().any(|c| c.contains("dark mode")),
        "assistant dark mode response should be present: {:?}",
        contents
    );

    // Verify metadata: role is stored in metadata map
    let assistant_entries: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.metadata
                .get("role")
                .map(|r| r == "assistant")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        assistant_entries.len(),
        2,
        "should have 2 assistant entries"
    );

    // Verify session_id extracted
    for entry in &entries {
        assert_eq!(
            entry.session_id.as_deref(),
            Some("session-abc"),
            "session_id should be extracted from sessionId field"
        );
    }

    // Verify source
    for entry in &entries {
        assert_eq!(
            entry.source, "claude-code",
            "source should be 'claude-code'"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: Parser skips empty and whitespace-only messages
// ---------------------------------------------------------------------------

#[test]
fn test_auto_store_skips_empty_messages() {
    let parser = ClaudeCodeParser;
    let fake_path = Path::new("/fake/test.jsonl");

    let edge_cases = vec![
        // Empty string
        "",
        // Whitespace only
        "   \n\t  ",
        // Valid JSON but empty content
        r#"{"type":"assistant","message":{"role":"assistant","content":""},"timestamp":"2026-01-01T10:00:00Z","sessionId":"s1"}"#,
        // Valid JSON but no content field in message
        r#"{"type":"assistant","message":{"role":"assistant"},"timestamp":"2026-01-01T10:00:00Z"}"#,
        // Tool result — not user/assistant role
        r#"{"type":"tool_result","message":{"role":"tool","content":"some output"},"timestamp":"2026-01-01T10:00:00Z"}"#,
        // Malformed JSON
        r#"not json at all"#,
    ];

    let mut entries = Vec::new();
    for line in &edge_cases {
        if let Some(entry) = parser.parse_line(line, fake_path) {
            entries.push(entry);
        }
    }

    assert_eq!(
        entries.len(),
        0,
        "parser should return 0 entries for empty/invalid/non-user-assistant messages"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Parser handles content as array of text blocks (Claude Code format)
// ---------------------------------------------------------------------------

#[test]
fn test_auto_store_content_array_format() {
    let parser = ClaudeCodeParser;
    let fake_path = Path::new("/fake/test.jsonl");

    // Claude Code sometimes sends content as an array of content blocks
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"First text block"},{"type":"text","text":"Second text block"},{"type":"tool_use","id":"tool1","name":"bash","input":{}}]},"timestamp":"2026-01-01T10:00:00Z","sessionId":"s2"}"#;

    let entry = parser.parse_line(line, fake_path);
    assert!(entry.is_some(), "should parse content array format");

    let entry = entry.unwrap();
    // Text blocks should be joined
    assert!(
        entry.content.contains("First text block"),
        "should contain first text block: {}",
        entry.content
    );
    assert!(
        entry.content.contains("Second text block"),
        "should contain second text block: {}",
        entry.content
    );
    // tool_use block should be excluded (not type=text)
}

// ---------------------------------------------------------------------------
// Test 4: Parser handles missing optional fields gracefully
// ---------------------------------------------------------------------------

#[test]
fn test_auto_store_optional_fields() {
    let parser = ClaudeCodeParser;
    let fake_path = Path::new("/fake/test.jsonl");

    // Minimal valid message — no timestamp, sessionId, or cwd
    let line = r#"{"message":{"role":"user","content":"Minimal message"}}"#;

    let entry = parser.parse_line(line, fake_path);
    assert!(
        entry.is_some(),
        "should parse message without optional fields"
    );

    let entry = entry.unwrap();
    assert_eq!(entry.content, "Minimal message");
    assert!(
        entry.timestamp.is_none(),
        "timestamp should be None when missing"
    );
    assert!(
        entry.session_id.is_none(),
        "session_id should be None when missing"
    );
    assert!(
        entry.project.is_none(),
        "project should be None when missing"
    );
}
