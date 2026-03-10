use memcp::auto_store::parser::create_parser;
use memcp::auto_store::parser::extract_agent_from_path;
use memcp::auto_store::parser::{ClaudeCodeParser, GenericJsonlParser, LogParser, OpenclawParser};
use std::path::Path;

#[test]
fn test_claude_code_parser_user_message() {
    let parser = ClaudeCodeParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"human","message":{"role":"user","content":"Remember to always use bun"},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123","cwd":"/home/user/project"}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "Remember to always use bun");
    assert_eq!(entry.source, "claude-code");
    assert!(entry.actor.is_none());
    assert_eq!(entry.session_id.as_deref(), Some("abc123"));
    assert_eq!(entry.project.as_deref(), Some("/home/user/project"));
    assert!(entry.timestamp.is_some());
}

#[test]
fn test_claude_code_parser_content_blocks() {
    let parser = ClaudeCodeParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Use pnpm for this project"},{"type":"tool_use","name":"bash"}]},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123"}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "Use pnpm for this project");
}

#[test]
fn test_claude_code_parser_skips_tool_only() {
    let parser = ClaudeCodeParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"bash"}]},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123"}"#;
    assert!(parser.parse_line(line, path).is_none());
}

#[test]
fn test_claude_code_parser_skips_system() {
    let parser = ClaudeCodeParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"system","message":{"role":"system","content":"system prompt"},"timestamp":"2025-01-15T10:30:00Z"}"#;
    assert!(parser.parse_line(line, path).is_none());
}

#[test]
fn test_openclaw_parser_assistant_message() {
    let parser = OpenclawParser;
    let path = Path::new("/Users/test/.openclaw/agents/vita/sessions/abc.jsonl");
    let line = r#"{"type":"message","id":"bb58ca3f","parentId":"2aa08748","timestamp":"2026-02-17T17:45:00.192Z","message":{"role":"assistant","content":[{"type":"text","text":"I'll check the webhook handler now"}]}}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "I'll check the webhook handler now");
    assert_eq!(entry.source, "openclaw/vita");
    assert_eq!(entry.actor.as_deref(), Some("vita"));
    assert_eq!(entry.project.as_deref(), Some("openclaw/vita"));
}

#[test]
fn test_openclaw_parser_user_message() {
    let parser = OpenclawParser;
    let path = Path::new("/Users/test/.openclaw/agents/main/sessions/abc.jsonl");
    let line = r#"{"type":"message","id":"aa11bb22","timestamp":"2026-02-17T17:45:00.192Z","message":{"role":"user","content":[{"type":"text","text":"check the build status"}]}}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "check the build status");
    assert_eq!(entry.source, "openclaw/main");
    assert!(entry.actor.is_none()); // user messages have no actor
}

#[test]
fn test_openclaw_parser_skips_tool_result() {
    let parser = OpenclawParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"message","id":"cc33","timestamp":"2026-02-17T17:45:00Z","message":{"role":"toolResult","content":[{"type":"text","text":"tool output"}]}}"#;
    assert!(parser.parse_line(line, path).is_none());
}

#[test]
fn test_openclaw_parser_skips_session_header() {
    let parser = OpenclawParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"type":"session","version":3,"id":"abc","timestamp":"2026-02-17T17:45:00Z"}"#;
    assert!(parser.parse_line(line, path).is_none());
}

#[test]
fn test_extract_agent_from_path() {
    assert_eq!(
        extract_agent_from_path(Path::new(
            "/Users/test/.openclaw/agents/vita/sessions/abc.jsonl"
        )),
        Some("vita".to_string())
    );
    assert_eq!(
        extract_agent_from_path(Path::new(
            "/home/user/.openclaw/agents/main/sessions/xyz.jsonl"
        )),
        Some("main".to_string())
    );
    assert_eq!(
        extract_agent_from_path(Path::new("/tmp/random/file.jsonl")),
        None
    );
}

#[test]
fn test_generic_parser() {
    let parser = GenericJsonlParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"content":"Some fact to remember","timestamp":"2025-01-15T10:30:00Z","session_id":"s1"}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "Some fact to remember");
    assert_eq!(entry.source, "generic");
}

#[test]
fn test_generic_parser_epoch_ms() {
    let parser = GenericJsonlParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"text":"hello","timestamp":1705315800000}"#;
    let entry = parser.parse_line(line, path).unwrap();
    assert_eq!(entry.content, "hello");
    assert!(entry.timestamp.is_some());
}

#[test]
fn test_generic_parser_empty_content() {
    let parser = GenericJsonlParser;
    let path = Path::new("/tmp/test.jsonl");
    let line = r#"{"content":""}"#;
    assert!(parser.parse_line(line, path).is_none());
}

#[test]
fn test_create_parser_known_formats() {
    let p1 = create_parser("claude-code");
    assert_eq!(p1.format_name(), "claude-code");
    let p2 = create_parser("openclaw");
    assert_eq!(p2.format_name(), "openclaw");
    let p3 = create_parser("generic-jsonl");
    assert_eq!(p3.format_name(), "generic-jsonl");
    let p4 = create_parser("auto");
    assert_eq!(p4.format_name(), "auto");
    let p5 = create_parser("unknown");
    assert_eq!(p5.format_name(), "auto");
}
