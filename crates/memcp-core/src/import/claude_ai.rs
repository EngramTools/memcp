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
    chatgpt::{MAX_DECOMPRESSED_SIZE, MAX_ZIP_ENTRIES},
    DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind,
};

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
        // SEC-05: Validate entry names against path traversal before processing.
        let mut json_entries: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();

            // Path traversal check (SEC-05)
            if !super::security::is_safe_zip_entry_name(&name) {
                tracing::warn!(
                    entry = %name,
                    "Skipping ZIP entry with unsafe path (possible path traversal)"
                );
                continue;
            }

            // Per-file size check (SEC-05)
            if entry.size() > super::security::MAX_SINGLE_FILE_SIZE {
                tracing::warn!(
                    entry = %name,
                    size = entry.size(),
                    max = super::security::MAX_SINGLE_FILE_SIZE,
                    "Skipping ZIP entry exceeding per-file size limit"
                );
                continue;
            }

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

            // Phase 24.75 D-01: one memory per message/turn (not per conversation).
            let messages = flatten_claude_messages(&conv);
            if messages.is_empty() {
                continue;
            }

            let base_tags = vec!["imported".to_string(), "imported:claude".to_string()];

            if opts.curate {
                // Curate mode: pass the joined conversation to the LLM as a single chunk.
                let joined = messages
                    .iter()
                    .map(|(role, text)| {
                        let label = if role == "human" { "User" } else { "Assistant" };
                        format!("{}: {}", label, text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let full = format!("# {}\n\n{}", title, joined);
                chunks.push(ImportChunk {
                    content: full,
                    type_hint: None, // LLM decides in curate.rs
                    source: "imported:claude".to_string(),
                    tags: base_tags,
                    created_at,
                    actor: None,
                    embedding: None,
                    embedding_model: None,
                    project: opts.project.clone(),
                });
            } else {
                // Default (Phase 24.75 D-01): one ImportChunk per message.
                for (role, text) in messages {
                    let mut tags = base_tags.clone();
                    tags.push(format!("role:{}", role));
                    chunks.push(ImportChunk {
                        content: text,
                        type_hint: Some("observation".to_string()),
                        source: "imported:claude".to_string(),
                        tags,
                        created_at,
                        actor: Some(role),
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

/// Phase 24.75 D-01: extract one `(role, text)` pair per surviving message.
///
/// Empty-text messages are dropped. Role is normalized to "human" or "assistant"
/// (other values pass through as the raw string — we don't try to interpret them).
fn flatten_claude_messages(conv: &ClaudeConversation) -> Vec<(String, String)> {
    let messages = match &conv.chat_messages {
        Some(m) if !m.is_empty() => m,
        _ => return Vec::new(),
    };

    let mut out = Vec::with_capacity(messages.len());
    for msg in messages {
        let text = msg.text.as_deref().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }
        let role = msg.sender.as_deref().unwrap_or("unknown").to_string();
        out.push((role, text.to_string()));
    }
    out
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
        assert!(flatten_claude_messages(&conv).is_empty());
    }

    /// Phase 24.75 D-01: two messages in → two `(role, text)` pairs out.
    /// No conversation-level concatenation, no chunk fan-out.
    #[test]
    fn test_flatten_conversation_per_message() {
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
        let msgs = flatten_claude_messages(&conv);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].0, "human");
        assert_eq!(msgs[0].1, "Hello, Claude!");
        assert_eq!(msgs[1].0, "assistant");
        assert_eq!(msgs[1].1, "Hi! How can I help you today?");
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
