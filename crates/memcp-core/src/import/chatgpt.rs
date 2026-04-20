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
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Maximum number of entries allowed in an import ZIP (ZIP bomb protection).
pub const MAX_ZIP_ENTRIES: usize = 10_000;

/// Maximum total decompressed size allowed in an import ZIP (ZIP bomb protection).
/// Checked via stored size metadata — no actual extraction is performed.
pub const MAX_DECOMPRESSED_SIZE: u64 = 500 * 1024 * 1024; // 500MB

// ── JSON structures for conversations.json ────────────────────────────────────

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // JSON schema struct retained for documentation purposes
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
    #[allow(dead_code)] // Present in JSON schema, not needed for import logic
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

        // Find conversations.json — do not assume it is entry 0.
        // SEC-05: Validate entry names against path traversal before processing.
        let conversations_json = {
            let mut found = None;
            for i in 0..archive.len() {
                let entry = archive.by_index(i)?;
                let entry_name = entry.name().to_string();

                // Path traversal check (SEC-05)
                if !super::security::is_safe_zip_entry_name(&entry_name) {
                    tracing::warn!(
                        entry = %entry_name,
                        "Skipping ZIP entry with unsafe path (possible path traversal)"
                    );
                    continue;
                }

                // Per-file size check (SEC-05)
                if entry.size() > super::security::MAX_SINGLE_FILE_SIZE {
                    tracing::warn!(
                        entry = %entry_name,
                        size = entry.size(),
                        max = super::security::MAX_SINGLE_FILE_SIZE,
                        "Skipping ZIP entry exceeding per-file size limit"
                    );
                    continue;
                }

                if entry_name == "conversations.json" || entry_name.ends_with("/conversations.json")
                {
                    drop(entry);
                    // Re-open by name to read contents.
                    let mut entry = archive.by_name(&entry_name)?;
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
            let created_at = conv
                .create_time
                .and_then(|ts| Utc.timestamp_opt(ts as i64, 0).single());

            // Apply --since filter per conversation.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            // Phase 24.75 D-01: one memory per message/turn (not per conversation).
            let messages = flatten_conversation(conv.mapping.as_ref());
            if messages.is_empty() {
                continue;
            }

            let conv_tag = format!("conversation:{}", sanitize_tag(&title));

            if opts.curate {
                // Curate mode: pass the joined conversation text to the LLM as a single
                // chunk so it can decide whether to keep the exchange intact. One chunk
                // per conversation is the correct atomic unit for curation.
                let joined = messages
                    .iter()
                    .map(|m| format!("{}: {}", label_for_role(&m.role), m.text))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let full = format!("# {}\n\n{}", title, joined);
                let tags = vec![
                    "imported".to_string(),
                    "imported:chatgpt".to_string(),
                    conv_tag,
                ];
                chunks.push(ImportChunk {
                    content: full,
                    type_hint: None, // LLM decides in curate.rs
                    source: "imported:chatgpt".to_string(),
                    tags,
                    created_at,
                    actor: None,
                    embedding: None,
                    embedding_model: None,
                    project: opts.project.clone(),
                });
            } else {
                // Default (Phase 24.75 D-01): one ImportChunk per message/turn.
                for msg in messages {
                    let tags = vec![
                        "imported".to_string(),
                        "imported:chatgpt".to_string(),
                        conv_tag.clone(),
                        format!("role:{}", msg.role),
                    ];
                    chunks.push(ImportChunk {
                        content: msg.text,
                        type_hint: Some("observation".to_string()),
                        source: "imported:chatgpt".to_string(),
                        tags,
                        created_at,
                        actor: Some(msg.role.clone()),
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

/// A single extracted message from a ChatGPT conversation (Phase 24.75 D-01).
pub(crate) struct FlatMessage {
    /// Normalized role: "user" or "assistant" (system + tool are filtered out).
    pub role: String,
    /// Message text with leading/trailing whitespace trimmed.
    pub text: String,
}

fn label_for_role(role: &str) -> &'static str {
    if role == "user" {
        "User"
    } else {
        "Assistant"
    }
}

/// Walk the mapping's parent chain to produce an ordered per-message list.
///
/// Phase 24.75 D-01: returns one entry per surviving message. Callers decide
/// whether to emit one `ImportChunk` per message (default) or a joined block
/// for curation.
fn flatten_conversation(
    mapping: Option<&std::collections::HashMap<String, MappingNode>>,
) -> Vec<FlatMessage> {
    let mapping = match mapping {
        Some(m) => m,
        None => return Vec::new(),
    };

    // Find root node: a node whose parent is None or whose parent is not in the map.
    let root_id = mapping.iter().find_map(|(id, node)| {
        let parent_missing = node
            .parent
            .as_ref()
            .map(|p| !mapping.contains_key(p.as_str()))
            .unwrap_or(true);
        if parent_missing {
            Some(id.clone())
        } else {
            None
        }
    });

    let mut ordered: Vec<FlatMessage> = Vec::new();

    let push_if_valid = |msg: &Message, out: &mut Vec<FlatMessage>| {
        if let Some(text) = extract_message_text(msg) {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            let role = msg
                .author
                .as_ref()
                .and_then(|a| a.role.as_deref())
                .unwrap_or("unknown");
            // Skip system and tool roles (not user-authored or assistant-authored turns).
            if matches!(role, "system" | "tool") {
                return;
            }
            out.push(FlatMessage {
                role: role.to_string(),
                text: trimmed.to_string(),
            });
        }
    };

    // DFS from root following children.
    if let Some(root) = root_id {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if let Some(node) = mapping.get(&id) {
                if let Some(msg) = &node.message {
                    push_if_valid(msg, &mut ordered);
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
                push_if_valid(msg, &mut ordered);
            }
        }
    }

    ordered
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
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_tag() {
        // Trailing punctuation is trimmed (trim_matches('-') removes boundary dashes).
        assert_eq!(sanitize_tag("Hello World!"), "hello-world");
        assert_eq!(sanitize_tag("Rust Programming"), "rust-programming");
        // Interior non-alphanumeric chars become dashes.
        assert_eq!(sanitize_tag("AI & ML"), "ai---ml");
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

    /// Phase 24.75 D-01: per-message granularity. Six valid messages ⇒ six chunks.
    #[test]
    fn test_flatten_conversation_per_message() {
        use std::collections::HashMap;
        let mut mapping: HashMap<String, MappingNode> = HashMap::new();
        // Build a 3-turn conversation (user → assistant → user → assistant → user → assistant)
        // as a linear chain so DFS walks in order.
        let turns = [
            ("user", "First user message long enough to pass any noise filter."),
            ("assistant", "First assistant reply long enough to count as signal content."),
            ("user", "Second user question also long enough to count as content."),
            ("assistant", "Second assistant reply long enough to count as content."),
            ("user", "Third user follow-up long enough to count as a real turn."),
            ("assistant", "Third assistant response long enough to be a turn."),
        ];
        let ids: Vec<String> = (0..turns.len()).map(|i| format!("n{}", i)).collect();
        for (i, (role, text)) in turns.iter().enumerate() {
            let parent = if i == 0 { None } else { Some(ids[i - 1].clone()) };
            let children = if i + 1 < turns.len() {
                Some(vec![ids[i + 1].clone()])
            } else {
                None
            };
            mapping.insert(
                ids[i].clone(),
                MappingNode {
                    message: Some(Message {
                        id: Some(ids[i].clone()),
                        author: Some(Author {
                            role: Some(role.to_string()),
                        }),
                        content: Some(MessageContent {
                            content_type: Some("text".to_string()),
                            parts: Some(vec![serde_json::Value::String(text.to_string())]),
                        }),
                    }),
                    parent,
                    children,
                },
            );
        }

        let msgs = flatten_conversation(Some(&mapping));
        assert_eq!(msgs.len(), 6, "one entry per turn, system/tool filtered");
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    /// CHUNK-05 acceptance: a single-message conversation of any size produces
    /// exactly one `ImportChunk` (no char-window fan-out).
    #[test]
    fn test_no_chunk_fanout_in_chatgpt_import() {
        use std::collections::HashMap;
        let huge = "x".repeat(30_000);
        let mut mapping: HashMap<String, MappingNode> = HashMap::new();
        mapping.insert(
            "root".to_string(),
            MappingNode {
                message: Some(Message {
                    id: Some("root".to_string()),
                    author: Some(Author {
                        role: Some("user".to_string()),
                    }),
                    content: Some(MessageContent {
                        content_type: Some("text".to_string()),
                        parts: Some(vec![serde_json::Value::String(huge.clone())]),
                    }),
                }),
                parent: None,
                children: None,
            },
        );

        let msgs = flatten_conversation(Some(&mapping));
        assert_eq!(msgs.len(), 1, "one message in, one message out — no char-window split");
        assert_eq!(msgs[0].text.len(), huge.len());
    }

    #[test]
    fn test_flatten_conversation_skips_system_and_tool() {
        use std::collections::HashMap;
        let mut mapping: HashMap<String, MappingNode> = HashMap::new();
        for (i, role) in ["system", "tool", "user", "assistant"].iter().enumerate() {
            mapping.insert(
                format!("n{}", i),
                MappingNode {
                    message: Some(Message {
                        id: None,
                        author: Some(Author {
                            role: Some(role.to_string()),
                        }),
                        content: Some(MessageContent {
                            content_type: Some("text".to_string()),
                            parts: Some(vec![serde_json::Value::String(
                                format!("{}-content-long-enough", role),
                            )]),
                        }),
                    }),
                    parent: if i == 0 { None } else { Some(format!("n{}", i - 1)) },
                    children: if i + 1 < 4 {
                        Some(vec![format!("n{}", i + 1)])
                    } else {
                        None
                    },
                },
            );
        }
        let msgs = flatten_conversation(Some(&mapping));
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().any(|m| m.role == "user"));
        assert!(msgs.iter().any(|m| m.role == "assistant"));
    }
}
