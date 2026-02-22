/// Log parser trait and implementations for auto-store sidecar.
///
/// Pluggable parser interface — ships with Claude Code and Openclaw parsers,
/// easy to add more formats.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;

/// A parsed log entry ready for filtering and storage.
#[derive(Debug, Clone)]
pub struct ParsedEntry {
    /// The text content to store as a memory
    pub content: String,
    /// When this entry was logged
    pub timestamp: Option<DateTime<Utc>>,
    /// Source identifier (e.g. "claude-code", "openclaw")
    pub source: String,
    /// Actor name (e.g. agent name for assistant messages, None for user messages)
    pub actor: Option<String>,
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
    ///
    /// `file_path` is the source file path — parsers can use it to extract context
    /// (e.g. agent name from directory structure).
    fn parse_line(&self, line: &str, file_path: &Path) -> Option<ParsedEntry>;

    /// The format name this parser handles (e.g. "claude-code").
    fn format_name(&self) -> &str;
}

/// Parser for Claude Code JSONL conversation logs.
///
/// Expected format: `{"type":"...", "message":{"role":"...", "content":"..."}, "timestamp":"...", "sessionId":"...", "cwd":"..."}`
/// Only extracts entries where role is "user" or "assistant" with non-empty text content.
pub struct ClaudeCodeParser;

impl LogParser for ClaudeCodeParser {
    fn parse_line(&self, line: &str, _file_path: &Path) -> Option<ParsedEntry> {
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
            actor: None,
            session_id,
            project,
            metadata,
        })
    }

    fn format_name(&self) -> &str {
        "claude-code"
    }
}

/// Parser for Openclaw JSONL conversation logs.
///
/// Expected format: `{"type":"message", "message":{"role":"user"|"assistant", "content":[...]}, "timestamp":"...", "id":"..."}`
/// Extracts user and assistant messages. Skips toolResult entries.
/// Agent name is inferred from the file path: `~/.openclaw/agents/<name>/sessions/<file>.jsonl`
pub struct OpenclawParser;

impl LogParser for OpenclawParser {
    fn parse_line(&self, line: &str, file_path: &Path) -> Option<ParsedEntry> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        let obj = value.as_object()?;

        // Openclaw uses type: "message" for all conversation entries
        let msg_type = obj.get("type")?.as_str()?;
        if msg_type != "message" {
            return None;
        }

        let message = obj.get("message")?;
        let role = message.get("role")?.as_str()?;

        // Only ingest user and assistant messages (skip toolResult)
        if role != "user" && role != "assistant" {
            return None;
        }

        // Content is array of content blocks (same as Claude Code)
        let content = extract_text_content(message.get("content")?)?;
        if content.is_empty() {
            return None;
        }

        let timestamp = obj
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let session_id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Infer agent name from path: .../agents/<name>/sessions/<file>.jsonl
        let agent_name = extract_agent_from_path(file_path);

        let mut metadata = HashMap::new();
        metadata.insert("role".to_string(), role.to_string());

        // Actor: assistant messages get the agent name, user messages get None
        let actor = if role == "assistant" {
            agent_name.clone()
        } else {
            None
        };

        // Source: "openclaw/<agent>" so filtering by source returns both sides
        // of the conversation. Falls back to plain "openclaw" if agent unknown.
        let source = match &agent_name {
            Some(name) => format!("openclaw/{}", name),
            None => "openclaw".to_string(),
        };

        let project = agent_name.map(|a| format!("openclaw/{}", a));

        Some(ParsedEntry {
            content,
            timestamp,
            source,
            actor,
            session_id,
            project,
            metadata,
        })
    }

    fn format_name(&self) -> &str {
        "openclaw"
    }
}

/// Extract agent name from file path.
/// Looks for pattern: .../agents/<name>/sessions/<file>.jsonl
fn extract_agent_from_path(path: &Path) -> Option<String> {
    let components: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Find "agents" component, then the next one is the agent name
    for (i, component) in components.iter().enumerate() {
        if *component == "agents" && i + 1 < components.len() {
            return Some(components[i + 1].to_string());
        }
    }
    None
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
    fn parse_line(&self, line: &str, _file_path: &Path) -> Option<ParsedEntry> {
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
            actor: None,
            session_id,
            project,
            metadata: HashMap::new(),
        })
    }

    fn format_name(&self) -> &str {
        "generic-jsonl"
    }
}

/// Auto-detecting parser that tries each known format.
///
/// Tries parsers in order: Claude Code → Openclaw → Generic JSONL.
/// Use `format = "auto"` in config to enable.
pub struct AutoDetectParser {
    parsers: Vec<Box<dyn LogParser>>,
}

impl AutoDetectParser {
    fn new() -> Self {
        Self {
            parsers: vec![
                Box::new(ClaudeCodeParser),
                Box::new(OpenclawParser),
                Box::new(GenericJsonlParser),
            ],
        }
    }
}

impl LogParser for AutoDetectParser {
    fn parse_line(&self, line: &str, file_path: &Path) -> Option<ParsedEntry> {
        for parser in &self.parsers {
            if let Some(entry) = parser.parse_line(line, file_path) {
                return Some(entry);
            }
        }
        None
    }

    fn format_name(&self) -> &str {
        "auto"
    }
}

/// Create a parser from a format name string.
pub fn create_parser(format: &str) -> Box<dyn LogParser> {
    match format {
        "claude-code" => Box::new(ClaudeCodeParser),
        "openclaw" => Box::new(OpenclawParser),
        "auto" => Box::new(AutoDetectParser::new()),
        "generic-jsonl" | "generic" => Box::new(GenericJsonlParser),
        other => {
            tracing::warn!(format = other, "Unknown auto-store format, falling back to auto-detect");
            Box::new(AutoDetectParser::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            extract_agent_from_path(Path::new("/Users/test/.openclaw/agents/vita/sessions/abc.jsonl")),
            Some("vita".to_string())
        );
        assert_eq!(
            extract_agent_from_path(Path::new("/home/user/.openclaw/agents/main/sessions/xyz.jsonl")),
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
}
