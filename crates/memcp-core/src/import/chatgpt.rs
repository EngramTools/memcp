//! ChatGPT export reader — imports conversations from ChatGPT ZIP exports.
//!
//! ChatGPT export format: `conversations.json` inside a ZIP file.
//! Each conversation has a title, create_time, and a `mapping` object
//! (uuid → message node). Messages are flattened by walking the parent chain.
//!
//! Export instructions: Settings > Data Controls > Export Data.
//! ChatGPT sends a ZIP file via email containing `conversations.json`.

use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Maximum chunk size in characters. Conversations longer than this are split.
const MAX_CHUNK_CHARS: usize = 2048;

// ── JSON structures for conversations.json ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatGptExport(Vec<Conversation>);

#[derive(Debug, Deserialize)]
struct Conversation {
    title: Option<String>,
    create_time: Option<f64>,
    mapping: Option<std::collections::HashMap<String, MappingNode>>,
}

#[derive(Debug, Deserialize)]
struct MappingNode {
    message: Option<Message>,
    parent: Option<String>,
    children: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Message {
    id: Option<String>,
    author: Option<Author>,
    content: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
struct Author {
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    content_type: Option<String>,
    parts: Option<Vec<Value>>,
}

// ── Reader ────────────────────────────────────────────────────────────────────

/// ChatGPT ZIP export reader. Implements `ImportSource` for ChatGPT conversation exports.
pub struct ChatGptReader;

#[async_trait]
impl ImportSource for ChatGptReader {
    fn source_name(&self) -> &str {
        "chatgpt"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::ChatGpt
    }

    /// Skip system messages, empty content, and very short (single-word) responses.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![
            // ChatGPT system/tool outputs commonly present in exports.
            "DALL-E displayed",
            "python_tool output",
            "browser_tool output",
        ]
    }

    /// ChatGPT exports must be manually requested — no auto-discovery.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        Ok(vec![DiscoveredSource {
            path: std::path::PathBuf::new(),
            source_type: "chatgpt".to_string(),
            item_count: 0,
            description: "Export from ChatGPT: Settings > Data Controls > Export Data. \
                          You will receive a ZIP file via email containing conversations.json."
                .to_string(),
        }])
    }

    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open ChatGPT ZIP at {:?}", path))?;

        let mut archive = zip::ZipArchive::new(BufReader::new(file))
            .with_context(|| "Failed to read ZIP archive")?;

        // Find conversations.json — do not assume it is entry 0.
        let conversations_json = {
            let mut found = None;
            for i in 0..archive.len() {
                let entry = archive.by_index(i)?;
                if entry.name() == "conversations.json"
                    || entry.name().ends_with("/conversations.json")
                {
                    let name = entry.name().to_string();
                    drop(entry);
                    // Re-open by name to read contents.
                    let mut entry = archive.by_name(&name)?;
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf)?;
                    found = Some(buf);
                    break;
                }
            }
            found.ok_or_else(|| anyhow!("conversations.json not found in ZIP archive"))?
        };

        let conversations: Vec<Conversation> = serde_json::from_slice(&conversations_json)
            .with_context(|| "Failed to parse conversations.json")?;

        let mut chunks = Vec::new();

        for conv in conversations {
            let title = conv.title.unwrap_or_else(|| "Untitled".to_string());
            let created_at = conv.create_time.and_then(|ts| {
                Utc.timestamp_opt(ts as i64, 0).single()
            });

            // Apply --since filter per conversation.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            let conversation_text = flatten_conversation(conv.mapping.as_ref(), &title);
            if conversation_text.trim().is_empty() {
                continue;
            }

            let tags = vec![
                "imported".to_string(),
                "imported:chatgpt".to_string(),
                format!("conversation:{}", sanitize_tag(&title)),
            ];

            if opts.curate {
                // Curate mode: pass full conversation to LLM later. One chunk per conversation.
                chunks.push(ImportChunk {
                    content: conversation_text,
                    type_hint: None, // LLM decides in curate.rs
                    source: "imported:chatgpt".to_string(),
                    tags,
                    created_at,
                    actor: None,
                    embedding: None,
                    embedding_model: None,
                    workspace: opts.project.clone(),
                });
            } else {
                // Default: chunk long conversations into <=2048-char pieces.
                let content_chunks = chunk_content(&conversation_text, MAX_CHUNK_CHARS);
                let total = content_chunks.len();
                for (i, piece) in content_chunks.into_iter().enumerate() {
                    let mut chunk_tags = tags.clone();
                    if total > 1 {
                        chunk_tags.push(format!("chunk:{}/{}", i + 1, total));
                    }
                    chunks.push(ImportChunk {
                        content: piece,
                        type_hint: Some("observation".to_string()),
                        source: "imported:chatgpt".to_string(),
                        tags: chunk_tags,
                        created_at,
                        actor: None,
                        embedding: None,
                        embedding_model: None,
                        workspace: opts.project.clone(),
                    });
                }
            }
        }

        Ok(chunks)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Walk the mapping's parent chain to produce an ordered message list,
/// then concatenate with role prefixes.
fn flatten_conversation(
    mapping: Option<&std::collections::HashMap<String, MappingNode>>,
    title: &str,
) -> String {
    let mapping = match mapping {
        Some(m) => m,
        None => return String::new(),
    };

    // Find root node: a node whose parent is None or whose parent is not in the map.
    let root_id = mapping.iter().find_map(|(id, node)| {
        let parent_missing = node.parent.as_ref().map(|p| !mapping.contains_key(p.as_str())).unwrap_or(true);
        if parent_missing {
            Some(id.clone())
        } else {
            None
        }
    });

    let mut ordered: Vec<String> = Vec::new();

    // DFS from root following children.
    if let Some(root) = root_id {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if let Some(node) = mapping.get(&id) {
                if let Some(msg) = &node.message {
                    if let Some(text) = extract_message_text(msg) {
                        if !text.trim().is_empty() {
                            let role = msg.author.as_ref()
                                .and_then(|a| a.role.as_deref())
                                .unwrap_or("unknown");
                            // Skip system and tool roles.
                            if !matches!(role, "system" | "tool") {
                                let label = if role == "user" { "User" } else { "Assistant" };
                                ordered.push(format!("{}: {}", label, text.trim()));
                            }
                        }
                    }
                }
                // Push children in reverse so first child is processed first.
                if let Some(children) = &node.children {
                    for child in children.iter().rev() {
                        stack.push(child.clone());
                    }
                }
            }
        }
    } else {
        // Fallback: insertion order (no clear root found).
        for node in mapping.values() {
            if let Some(msg) = &node.message {
                if let Some(text) = extract_message_text(msg) {
                    if !text.trim().is_empty() {
                        let role = msg.author.as_ref()
                            .and_then(|a| a.role.as_deref())
                            .unwrap_or("unknown");
                        if !matches!(role, "system" | "tool") {
                            let label = if role == "user" { "User" } else { "Assistant" };
                            ordered.push(format!("{}: {}", label, text.trim()));
                        }
                    }
                }
            }
        }
    }

    if ordered.is_empty() {
        return String::new();
    }

    format!("# {}\n\n{}", title, ordered.join("\n\n"))
}

/// Extract text from a ChatGPT message content object.
fn extract_message_text(msg: &Message) -> Option<String> {
    let content = msg.content.as_ref()?;
    // Only handle text content types.
    let ct = content.content_type.as_deref().unwrap_or("text");
    if ct != "text" && ct != "tether_browsing_display" && ct != "tether_quote" {
        // Skip code, image, tool_use, etc. for now.
        // Future: extract code blocks from code content type.
        if ct != "code" {
            return None;
        }
    }
    let parts = content.parts.as_ref()?;
    let mut text_parts: Vec<String> = Vec::new();
    for part in parts {
        match part {
            Value::String(s) if !s.trim().is_empty() => text_parts.push(s.clone()),
            Value::Object(obj) => {
                // Some parts are objects with a "text" field.
                if let Some(Value::String(t)) = obj.get("text") {
                    if !t.trim().is_empty() {
                        text_parts.push(t.clone());
                    }
                }
            }
            _ => {}
        }
    }
    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    }
}

/// Sanitize a conversation title for use as a tag (lowercase, replace spaces and punctuation).
fn sanitize_tag(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Split content into chunks of at most `max_chars` characters.
/// Tries to split at sentence boundaries (period+space) when possible.
pub fn chunk_content(content: &str, max_chars: usize) -> Vec<String> {
    if content.len() <= max_chars {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;

    while remaining.len() > max_chars {
        // Try to find a sentence boundary within the max window.
        let window = &remaining[..max_chars];
        let split_pos = window
            .rfind(". ")
            .or_else(|| window.rfind('\n'))
            .unwrap_or(max_chars);

        let (chunk, rest) = remaining.split_at(split_pos + 1);
        chunks.push(chunk.trim().to_string());
        remaining = rest.trim_start();
    }

    if !remaining.trim().is_empty() {
        chunks.push(remaining.trim().to_string());
    }

    chunks
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_tag() {
        assert_eq!(sanitize_tag("Hello World!"), "hello-world-");
        assert_eq!(sanitize_tag("Rust Programming"), "rust-programming");
        assert_eq!(sanitize_tag("AI & ML"), "ai---ml");
    }

    #[test]
    fn test_chunk_content_short() {
        let chunks = chunk_content("Short content.", 2048);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Short content.");
    }

    #[test]
    fn test_chunk_content_long() {
        // Build a string longer than 2048 chars.
        let long = "Word. ".repeat(500); // 3000 chars
        let chunks = chunk_content(&long, 2048);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= 2048 + 20, "chunk too long: {}", c.len());
        }
    }

    #[test]
    fn test_source_name() {
        let reader = ChatGptReader;
        assert_eq!(reader.source_name(), "chatgpt");
        assert_eq!(reader.source_kind(), ImportSourceKind::ChatGpt);
    }

    #[tokio::test]
    async fn test_reader_rejects_non_zip() {
        let reader = ChatGptReader;
        let opts = ImportOpts::default();
        let result = reader
            .read_chunks(std::path::Path::new("/nonexistent/file.zip"), &opts)
            .await;
        assert!(result.is_err());
    }
}
