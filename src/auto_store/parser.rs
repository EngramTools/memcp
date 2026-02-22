/// Log parser trait and implementations for auto-store sidecar.
///
/// Pluggable parser interface — ships with Claude Code JSONL parser,
/// easy to add more formats.

use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A parsed log entry ready for filtering and storage.
#[derive(Debug, Clone)]
pub struct ParsedEntry {
    /// The text content to store as a memory
    pub content: String,
    /// When this entry was logged
    pub timestamp: Option<DateTime<Utc>>,
    /// Source identifier (e.g. "claude-code", "generic")
    pub source: String,
    /// Session identifier from the log format
    pub session_id: Option<String>,
    /// Project path or name from the log format
    pub project: Option<String>,
    /// Additional key-value metadata
    pub metadata: HashMap<String, String>,
}

/// Trait for parsing log file lines into structured entries.
///
/// Implementations must be Send + Sync for use in async contexts.
pub trait LogParser: Send + Sync {
    /// Attempt to parse a single line. Returns None if the line is not relevant
    /// (e.g. empty, malformed, or a non-content entry like a tool call).
    fn parse_line(&self, line: &str) -> Option<ParsedEntry>;

    /// The format name this parser handles (e.g. "claude-code").
    fn format_name(&self) -> &str;
}

/// Parser for Claude Code JSONL conversation logs.
///
/// Expected format: `{"type":"...", "message":{"role":"...", "content":"..."}, "timestamp":"...", "sessionId":"...", "cwd":"..."}`
/// Only extracts entries where role is "user" or "assistant" with non-empty text content.
pub struct ClaudeCodeParser;

impl LogParser for ClaudeCodeParser {
    fn parse_line(&self, line: &str) -> Option<ParsedEntry> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let obj = value.as_object()?;

        // Extract the message content — Claude Code uses nested message.content
        let message = obj.get("message")?;
        let role = message.get("role")?.as_str()?;

        // Only ingest user and assistant messages
        if role != "user" && role != "assistant" {
            return None;
        }

        // Content can be a string or array of content blocks
        let content = extract_text_content(message.get("content")?)?;
        if content.is_empty() {
            return None;
        }

        // Parse timestamp — Claude Code uses ISO 8601 string
        let timestamp = obj
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let session_id = obj
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let project = obj
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut metadata = HashMap::new();
        metadata.insert("role".to_string(), role.to_string());
        if let Some(msg_type) = obj.get("type").and_then(|v| v.as_str()) {
            metadata.insert("type".to_string(), msg_type.to_string());
        }

        Some(ParsedEntry {
            content,
            timestamp,
            source: "claude-code".to_string(),
            session_id,
            project,
            metadata,
        })
    }

    fn format_name(&self) -> &str {
        "claude-code"
    }
}

/// Extract text content from a JSON value that may be a string or array of content blocks.
fn extract_text_content(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => {
            let texts: Vec<&str> = blocks
                .iter()
                .filter_map(|block| {
                    if block.get("type")?.as_str()? == "text" {
                        block.get("text")?.as_str()
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

/// Generic JSONL parser — expects `{"content":"...", "timestamp":"...", ...}`.
///
/// Minimal format for interop with custom tools.
pub struct GenericJsonlParser;

impl LogParser for GenericJsonlParser {
    fn parse_line(&self, line: &str) -> Option<ParsedEntry> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let obj = value.as_object()?;

        // "content" or "text" or "display" field
        let content = obj
            .get("content")
            .or_else(|| obj.get("text"))
            .or_else(|| obj.get("display"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?;

        if content.is_empty() {
            return None;
        }

        let timestamp = obj
            .get("timestamp")
            .and_then(|t| {
                // Try string first, then epoch ms
                if let Some(s) = t.as_str() {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                } else if let Some(ms) = t.as_i64() {
                    DateTime::from_timestamp_millis(ms)
                } else {
                    None
                }
            });

        let session_id = obj
            .get("sessionId")
            .or_else(|| obj.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let project = obj
            .get("project")
            .or_else(|| obj.get("cwd"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(ParsedEntry {
            content,
            timestamp,
            source: "generic".to_string(),
            session_id,
            project,
            metadata: HashMap::new(),
        })
    }

    fn format_name(&self) -> &str {
        "generic-jsonl"
    }
}

/// Create a parser from a format name string.
pub fn create_parser(format: &str) -> Box<dyn LogParser> {
    match format {
        "claude-code" => Box::new(ClaudeCodeParser),
        "generic-jsonl" | "generic" => Box::new(GenericJsonlParser),
        other => {
            tracing::warn!(format = other, "Unknown auto-store format, falling back to generic-jsonl");
            Box::new(GenericJsonlParser)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_code_parser_user_message() {
        let parser = ClaudeCodeParser;
        let line = r#"{"type":"human","message":{"role":"user","content":"Remember to always use bun"},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123","cwd":"/home/user/project"}"#;
        let entry = parser.parse_line(line).unwrap();
        assert_eq!(entry.content, "Remember to always use bun");
        assert_eq!(entry.source, "claude-code");
        assert_eq!(entry.session_id.as_deref(), Some("abc123"));
        assert_eq!(entry.project.as_deref(), Some("/home/user/project"));
        assert!(entry.timestamp.is_some());
    }

    #[test]
    fn test_claude_code_parser_content_blocks() {
        let parser = ClaudeCodeParser;
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Use pnpm for this project"},{"type":"tool_use","name":"bash"}]},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123"}"#;
        let entry = parser.parse_line(line).unwrap();
        assert_eq!(entry.content, "Use pnpm for this project");
    }

    #[test]
    fn test_claude_code_parser_skips_tool_only() {
        let parser = ClaudeCodeParser;
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"bash"}]},"timestamp":"2025-01-15T10:30:00Z","sessionId":"abc123"}"#;
        assert!(parser.parse_line(line).is_none());
    }

    #[test]
    fn test_claude_code_parser_skips_system() {
        let parser = ClaudeCodeParser;
        let line = r#"{"type":"system","message":{"role":"system","content":"system prompt"},"timestamp":"2025-01-15T10:30:00Z"}"#;
        assert!(parser.parse_line(line).is_none());
    }

    #[test]
    fn test_generic_parser() {
        let parser = GenericJsonlParser;
        let line = r#"{"content":"Some fact to remember","timestamp":"2025-01-15T10:30:00Z","session_id":"s1"}"#;
        let entry = parser.parse_line(line).unwrap();
        assert_eq!(entry.content, "Some fact to remember");
        assert_eq!(entry.source, "generic");
    }

    #[test]
    fn test_generic_parser_epoch_ms() {
        let parser = GenericJsonlParser;
        let line = r#"{"text":"hello","timestamp":1705315800000}"#;
        let entry = parser.parse_line(line).unwrap();
        assert_eq!(entry.content, "hello");
        assert!(entry.timestamp.is_some());
    }

    #[test]
    fn test_generic_parser_empty_content() {
        let parser = GenericJsonlParser;
        let line = r#"{"content":""}"#;
        assert!(parser.parse_line(line).is_none());
    }

    #[test]
    fn test_create_parser_known_formats() {
        let p1 = create_parser("claude-code");
        assert_eq!(p1.format_name(), "claude-code");
        let p2 = create_parser("generic-jsonl");
        assert_eq!(p2.format_name(), "generic-jsonl");
        let p3 = create_parser("unknown");
        assert_eq!(p3.format_name(), "generic-jsonl");
    }
}
