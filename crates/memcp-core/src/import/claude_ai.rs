//! Claude.ai export reader — imports conversations from Claude.ai ZIP exports.
//!
//! Claude.ai export format: individual JSON files per conversation inside a ZIP,
//! OR a single conversations.json. Both formats are handled.
//!
//! Each conversation JSON contains a `chat_messages` array with `role` and `content`.
//!
//! Export instructions: Claude.ai > Settings > Export Data. Download the ZIP file.

use std::io::{BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{
    chatgpt::{chunk_content, MAX_DECOMPRESSED_SIZE, MAX_ZIP_ENTRIES},
    DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind,
};

/// Maximum chunk size in characters. Conversations longer than this are split.
const MAX_CHUNK_CHARS: usize = 2048;

// ── JSON structures ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ClaudeConversation {
    /// Conversation name/title.
    name: Option<String>,
    /// UUID or other identifier.
    uuid: Option<String>,
    /// ISO 8601 creation time.
    created_at: Option<String>,
    /// Messages in this conversation.
    chat_messages: Option<Vec<ClaudeMessage>>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    /// "human" or "assistant"
    sender: Option<String>,
    /// Message text.
    text: Option<String>,
    /// ISO 8601 creation time for this message.
    #[allow(dead_code)] // Present in JSON schema, not needed for import logic
    created_at: Option<String>,
}

// ── Reader ────────────────────────────────────────────────────────────────────

/// Claude.ai ZIP export reader. Implements `ImportSource` for Claude.ai conversation exports.
pub struct ClaudeAiReader;

#[async_trait]
impl ImportSource for ClaudeAiReader {
    fn source_name(&self) -> &str {
        "claude"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::ClaudeAi
    }

    /// Claude.ai exports are clean — no hardcoded noise patterns needed.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Claude.ai exports must be manually requested — no auto-discovery.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        Ok(vec![DiscoveredSource {
            path: std::path::PathBuf::new(),
            source_type: "claude".to_string(),
            item_count: 0,
            description: "Export from Claude.ai: Settings > Export Data. \
                          Download the ZIP file."
                .to_string(),
        }])
    }

    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open Claude.ai ZIP at {:?}", path))?;

        let mut archive = zip::ZipArchive::new(BufReader::new(file))
            .with_context(|| "Failed to read ZIP archive")?;

        // ZIP bomb protection: reject archives with too many entries or excessive decompressed size.
        let entry_count = archive.len();
        if entry_count > MAX_ZIP_ENTRIES {
            anyhow::bail!(
                "ZIP file has {} entries (max {}). This may be a ZIP bomb. \
                 If this is a legitimate file, contact support.",
                entry_count,
                MAX_ZIP_ENTRIES
            );
        }
        let total_size: u64 = (0..entry_count)
            .map(|i| archive.by_index(i).map(|f| f.size()).unwrap_or(0))
            .sum();
        if total_size > MAX_DECOMPRESSED_SIZE {
            anyhow::bail!(
                "ZIP decompressed size is {} bytes (max {} bytes / 500MB). \
                 This may be a ZIP bomb.",
                total_size,
                MAX_DECOMPRESSED_SIZE
            );
        }

        // Collect all JSON entry contents from the archive.
        // Claude.ai may ship a single conversations.json or per-conversation JSON files.
        let mut json_entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            if name.ends_with(".json") && !name.contains("__MACOSX") {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                json_entries.push((name, buf));
            }
        }

        let mut conversations: Vec<ClaudeConversation> = Vec::new();

        for (name, bytes) in json_entries {
            if name == "conversations.json" || name.ends_with("/conversations.json") {
                // Bulk format: array of conversations.
                match serde_json::from_slice::<Vec<ClaudeConversation>>(&bytes) {
                    Ok(convs) => conversations.extend(convs),
                    Err(e) => {
                        tracing::warn!("Failed to parse {}: {}", name, e);
                    }
                }
            } else {
                // Per-file format: single conversation object.
                match serde_json::from_slice::<ClaudeConversation>(&bytes) {
                    Ok(conv) => conversations.push(conv),
                    Err(e) => {
                        tracing::warn!("Failed to parse conversation file {}: {}", name, e);
                    }
                }
            }
        }

        let mut chunks = Vec::new();

        for conv in conversations {
            let title = conv
                .name
                .clone()
                .or_else(|| conv.uuid.clone())
                .unwrap_or_else(|| "Untitled".to_string());

            let created_at: Option<DateTime<Utc>> = conv
                .created_at
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            // Apply --since filter per conversation.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            let conversation_text = flatten_claude_conversation(&conv, &title);
            if conversation_text.trim().is_empty() {
                continue;
            }

            let tags = vec!["imported".to_string(), "imported:claude".to_string()];

            if opts.curate {
                // Curate mode: pass full conversation to LLM later.
                chunks.push(ImportChunk {
                    content: conversation_text,
                    type_hint: None, // LLM decides in curate.rs
                    source: "imported:claude".to_string(),
                    tags,
                    created_at,
                    actor: None,
                    embedding: None,
                    embedding_model: None,
                    project: opts.project.clone(),
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
                        source: "imported:claude".to_string(),
                        tags: chunk_tags,
                        created_at,
                        actor: None,
                        embedding: None,
                        embedding_model: None,
                        project: opts.project.clone(),
                    });
                }
            }
        }

        Ok(chunks)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Flatten a Claude.ai conversation into a single text block with role prefixes.
fn flatten_claude_conversation(conv: &ClaudeConversation, title: &str) -> String {
    let messages = match &conv.chat_messages {
        Some(m) if !m.is_empty() => m,
        _ => return String::new(),
    };

    let mut lines = Vec::new();
    for msg in messages {
        let text = msg.text.as_deref().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        let role = msg.sender.as_deref().unwrap_or("unknown");
        let label = if role == "human" { "User" } else { "Assistant" };
        lines.push(format!("{}: {}", label, text));
    }

    if lines.is_empty() {
        return String::new();
    }

    format!("# {}\n\n{}", title, lines.join("\n\n"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_name() {
        let reader = ClaudeAiReader;
        assert_eq!(reader.source_name(), "claude");
        assert_eq!(reader.source_kind(), ImportSourceKind::ClaudeAi);
    }

    #[test]
    fn test_flatten_empty_conversation() {
        let conv = ClaudeConversation {
            name: Some("Test".to_string()),
            uuid: None,
            created_at: None,
            chat_messages: None,
        };
        let text = flatten_claude_conversation(&conv, "Test");
        assert!(text.is_empty());
    }

    #[test]
    fn test_flatten_conversation_with_messages() {
        let conv = ClaudeConversation {
            name: Some("My Conversation".to_string()),
            uuid: None,
            created_at: None,
            chat_messages: Some(vec![
                ClaudeMessage {
                    sender: Some("human".to_string()),
                    text: Some("Hello, Claude!".to_string()),
                    created_at: None,
                },
                ClaudeMessage {
                    sender: Some("assistant".to_string()),
                    text: Some("Hi! How can I help you today?".to_string()),
                    created_at: None,
                },
            ]),
        };
        let text = flatten_claude_conversation(&conv, "My Conversation");
        assert!(text.contains("# My Conversation"));
        assert!(text.contains("User: Hello, Claude!"));
        assert!(text.contains("Assistant: Hi!"));
    }

    #[tokio::test]
    async fn test_reader_rejects_missing_file() {
        let reader = ClaudeAiReader;
        let opts = ImportOpts::default();
        let result = reader
            .read_chunks(std::path::Path::new("/nonexistent/export.zip"), &opts)
            .await;
        assert!(result.is_err());
    }
}
