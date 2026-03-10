//! Log parser trait and implementations for auto-store sidecar.
//!
//! Pluggable parser interface — ships with Claude Code and Openclaw parsers,
//! easy to add more formats.

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
/// Exposed as `pub` for external test access.
pub fn extract_agent_from_path(path: &Path) -> Option<String> {
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

        let timestamp = obj.get("timestamp").and_then(|t| {
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
            tracing::warn!(
                format = other,
                "Unknown auto-store format, falling back to auto-detect"
            );
            Box::new(AutoDetectParser::new())
        }
    }
}
