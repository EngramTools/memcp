//! Claude Code reader — imports MEMORY.md files and optionally history.jsonl.
//!
//! Claude Code stores per-project memory files at:
//!   ~/.claude/MEMORY.md                        (global)
//!   ~/.claude/projects/{slug}/memory/MEMORY.md (per-project)
//!
//! MEMORY.md files are user-curated summaries — high-signal, imported as type_hint=fact.
//! Section-based chunking (split on `#` and `##` headers) preserves semantic boundaries.
//!
//! history.jsonl is opt-in via --include-history and imports assistant responses
//! as type_hint=observation. Low-signal entries (tool calls, system messages, etc.)
//! are filtered via noise_patterns().

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::config::ChunkingConfig;
use crate::pipeline::chunking::chunk_content;

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Parsed line from Claude Code history.jsonl.
#[derive(Debug, Deserialize)]
struct HistoryLine {
    #[serde(rename = "type")]
    line_type: Option<String>,
    message: Option<serde_json::Value>,
    timestamp: Option<String>,
}

/// Reads memories from Claude Code MEMORY.md files and optionally history.jsonl.
pub struct ClaudeCodeReader {
    /// If true, also import assistant messages from history.jsonl.
    include_history: bool,
}

impl ClaudeCodeReader {
    /// Create a new ClaudeCodeReader.
    pub fn new(include_history: bool) -> Self {
        Self { include_history }
    }

    /// Default path to search for Claude Code data (~/.claude/).
    pub fn default_base_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude"))
    }

    /// Count section headers in a MEMORY.md file (approximate item count for discovery).
    fn count_sections(content: &str) -> usize {
        content
            .lines()
            .filter(|l| l.starts_with("# ") || l.starts_with("## "))
            .count()
            .max(1) // At least 1 if the file has any content
    }

    /// Extract project name from a Claude Code project directory slug.
    ///
    /// Claude Code encodes project paths as slugs like `-Users-foo-myproject`.
    /// We reverse this to get a human-readable project name.
    fn project_name_from_slug(slug: &str) -> String {
        // Slug format: -Users-foo-myproject → take last segment
        slug.trim_start_matches('-')
            .rsplit('-')
            .next()
            .unwrap_or(slug)
            .to_string()
    }

    /// Split MEMORY.md content into sections based on `#` and `##` headers.
    ///
    /// Each header + following content becomes one ImportChunk. If a section
    /// is very long (>2048 chars), it's further split using chunk_content().
    fn split_into_sections(content: &str, source_path: &str, project: Option<&str>) -> Vec<ImportChunk> {
        let mut chunks = Vec::new();
        let mut current_header: Option<String> = None;
        let mut current_lines: Vec<&str> = Vec::new();

        let flush_section = |header: &Option<String>,
                              lines: &[&str],
                              chunks: &mut Vec<ImportChunk>,
                              project: Option<&str>| {
            let body = lines.join("\n").trim().to_string();
            if body.is_empty() {
                return;
            }

            let content = if let Some(h) = header {
                format!("{}\n\n{}", h, body)
            } else {
                body.clone()
            };

            // For very long sections, use sentence-based chunking.
            let sub_chunks = if content.len() > 2048 {
                let cfg = ChunkingConfig {
                    enabled: true,
                    max_chunk_chars: 1024,
                    overlap_sentences: 1,
                    min_content_chars: 64,
                };
                let splits = chunk_content(&content, &cfg);
                if splits.is_empty() {
                    vec![content]
                } else {
                    splits.into_iter().map(|c| c.content).collect()
                }
            } else {
                vec![content]
            };

            for sub in sub_chunks {
                chunks.push(ImportChunk {
                    content: sub,
                    type_hint: Some("fact".to_string()),
                    source: "imported:claude-code".to_string(),
                    tags: vec!["imported".to_string(), "imported:claude-code".to_string()],
                    created_at: None,
                    actor: None,
                    embedding: None,
                    embedding_model: None,
                    project: project.map(|w| w.to_string()),
                });
            }
        };

        for line in content.lines() {
            if line.starts_with("## ") || line.starts_with("# ") {
                // Flush the previous section.
                flush_section(&current_header, &current_lines, &mut chunks, project);
                current_header = Some(line.to_string());
                current_lines = Vec::new();
            } else {
                current_lines.push(line);
            }
        }

        // Flush the last section.
        flush_section(&current_header, &current_lines, &mut chunks, project);

        // Tag with source path for traceability.
        for chunk in &mut chunks {
            chunk.tags.push(format!("source:{}", source_path));
        }

        chunks
    }

    /// Read a MEMORY.md file and return ImportChunks.
    fn read_memory_md(path: &Path, project: Option<&str>) -> Result<Vec<ImportChunk>> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read MEMORY.md at {:?}", path))?;

        if content.trim().is_empty() {
            return Ok(vec![]);
        }

        let source_path = path.to_string_lossy();
        Ok(Self::split_into_sections(&content, &source_path, project))
    }

    /// Read history.jsonl file and return ImportChunks for assistant messages.
    fn read_history_jsonl(path: &Path) -> Result<Vec<ImportChunk>> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("Failed to open history.jsonl at {:?}", path))?;
        let reader = BufReader::new(file);

        let mut chunks = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    warn!("history.jsonl line {}: read error: {}", line_num + 1, e);
                    continue;
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parsed: HistoryLine = match serde_json::from_str(trimmed) {
                Ok(p) => p,
                Err(e) => {
                    debug!("history.jsonl line {}: JSON parse error: {}", line_num + 1, e);
                    continue;
                }
            };

            // Only import assistant messages.
            if parsed.line_type.as_deref() != Some("assistant") {
                continue;
            }

            // Extract text content from the message field.
            let text = match extract_message_text(&parsed.message) {
                Some(t) if !t.trim().is_empty() => t,
                _ => continue,
            };

            let created_at = parsed.timestamp.as_deref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            });

            chunks.push(ImportChunk {
                content: text,
                type_hint: Some("observation".to_string()),
                source: "imported:claude-code".to_string(),
                tags: vec![
                    "imported".to_string(),
                    "imported:claude-code".to_string(),
                    "history".to_string(),
                ],
                created_at,
                actor: None,
                embedding: None,
                embedding_model: None,
                project: None,
            });
        }

        Ok(chunks)
    }
}

/// Extract plain text from a Claude history.jsonl message field.
///
/// The message field can be:
///   - A string
///   - An object with a "content" field (string or array of content blocks)
fn extract_message_text(message: &Option<serde_json::Value>) -> Option<String> {
    let msg = message.as_ref()?;

    match msg {
        serde_json::Value::String(s) => {
            if s.is_empty() { None } else { Some(s.clone()) }
        }
        serde_json::Value::Object(map) => {
            let content = map.get("content")?;
            match content {
                serde_json::Value::String(s) => {
                    if s.is_empty() { None } else { Some(s.clone()) }
                }
                serde_json::Value::Array(arr) => {
                    // Concatenate all text content blocks.
                    let text: String = arr.iter().filter_map(|block| {
                        if block.get("type")?.as_str()? == "text" {
                            block.get("text")?.as_str().map(|s| s.to_string())
                        } else {
                            None
                        }
                    }).collect::<Vec<_>>().join("\n");
                    if text.is_empty() { None } else { Some(text) }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[async_trait]
impl ImportSource for ClaudeCodeReader {
    fn source_name(&self) -> &str {
        "claude-code"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::ClaudeCode
    }

    /// Noise patterns for history.jsonl (MEMORY.md is already curated, needs no filtering).
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![
            "Let me ",
            "I'll ",
            "I will ",
            "I've ",
            "I have ",
            "LGTM",
            "lgtm",
        ]
    }

    /// Discover MEMORY.md files in Claude Code directories.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        let base = match Self::default_base_path() {
            Some(p) => p,
            None => return Ok(vec![]),
        };

        let mut sources = Vec::new();

        // Check global ~/.claude/MEMORY.md.
        let global_memory = base.join("MEMORY.md");
        if global_memory.exists() {
            let content = std::fs::read_to_string(&global_memory).unwrap_or_default();
            let count = Self::count_sections(&content);
            sources.push(DiscoveredSource {
                path: global_memory,
                source_type: "claude-code".to_string(),
                item_count: count,
                description: format!("Claude Code global MEMORY.md: ~{} sections", count),
            });
        }

        // Scan ~/.claude/projects/ for per-project MEMORY.md files.
        let projects_dir = base.join("projects");
        if projects_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&projects_dir) {
                for entry in entries.flatten() {
                    let project_path = entry.path();
                    if !project_path.is_dir() {
                        continue;
                    }

                    // Look for memory/MEMORY.md within each project directory.
                    let memory_file = project_path.join("memory").join("MEMORY.md");
                    if memory_file.exists() {
                        let slug = project_path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "unknown".to_string());
                        let project_name = Self::project_name_from_slug(&slug);
                        let content = std::fs::read_to_string(&memory_file).unwrap_or_default();
                        let count = Self::count_sections(&content);
                        sources.push(DiscoveredSource {
                            path: memory_file,
                            source_type: "claude-code".to_string(),
                            item_count: count,
                            description: format!(
                                "Claude Code project {}: ~{} sections",
                                project_name, count
                            ),
                        });
                    }
                }
            }
        }

        sources.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(sources)
    }

    /// Read chunks from a MEMORY.md file or directory of MEMORY.md files.
    async fn read_chunks(&self, path: &Path, _opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let include_history = self.include_history;
        let path = path.to_path_buf();

        let chunks = tokio::task::spawn_blocking(move || -> Result<Vec<ImportChunk>> {
            let mut all_chunks = Vec::new();

            if path.is_dir() {
                // Scan directory for MEMORY.md files and optionally history.jsonl.
                collect_memory_files(&path, &mut all_chunks, include_history)?;
            } else if path.extension().map(|e| e == "md").unwrap_or(false)
                || path.file_name().map(|f| f == "MEMORY.md").unwrap_or(false)
            {
                // Single MEMORY.md file.
                let project = extract_project_from_path(&path);
                let file_chunks = ClaudeCodeReader::read_memory_md(&path, project.as_deref())?;
                all_chunks.extend(file_chunks);
            } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                // Explicit history.jsonl file.
                if include_history {
                    let file_chunks = ClaudeCodeReader::read_history_jsonl(&path)?;
                    all_chunks.extend(file_chunks);
                }
            }

            Ok(all_chunks)
        })
        .await
        .with_context(|| "spawn_blocking task panicked")??;

        Ok(chunks)
    }
}

/// Recursively collect chunks from MEMORY.md (and history.jsonl) files in a directory.
fn collect_memory_files(
    dir: &Path,
    chunks: &mut Vec<ImportChunk>,
    include_history: bool,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("Cannot read directory {:?}: {}", dir, e);
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_memory_files(&path, chunks, include_history)?;
        } else if path.file_name().map(|f| f == "MEMORY.md").unwrap_or(false) {
            let project = extract_project_from_path(&path);
            match ClaudeCodeReader::read_memory_md(&path, project.as_deref()) {
                Ok(file_chunks) => chunks.extend(file_chunks),
                Err(e) => warn!("Failed to read {:?}: {}", path, e),
            }
        } else if include_history
            && path.file_name().map(|f| f == "history.jsonl").unwrap_or(false)
        {
            match ClaudeCodeReader::read_history_jsonl(&path) {
                Ok(file_chunks) => chunks.extend(file_chunks),
                Err(e) => warn!("Failed to read history {:?}: {}", path, e),
            }
        }
    }

    Ok(())
}

/// Try to determine a project name from a MEMORY.md file path.
///
/// `~/.claude/projects/-Users-foo-myproject/memory/MEMORY.md` → "myproject"
fn extract_project_from_path(path: &Path) -> Option<String> {
    // Walk up looking for the projects directory.
    let mut current = path.parent()?;
    loop {
        if current.file_name()?.to_string_lossy() == "memory" {
            // Parent of "memory" dir is the project slug.
            let slug = current.parent()?.file_name()?.to_string_lossy().into_owned();
            if slug.starts_with('-') {
                return Some(ClaudeCodeReader::project_name_from_slug(&slug));
            }
        }
        current = current.parent()?;
        // Stop at home dir boundary.
        if current.file_name().map(|f| f == ".claude").unwrap_or(false) {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_count_sections_basic() {
        let content = "# Section 1\nSome content\n## Subsection\nMore\n# Section 2\nEnd";
        assert_eq!(ClaudeCodeReader::count_sections(content), 3);
    }

    #[test]
    fn test_count_sections_empty() {
        assert_eq!(ClaudeCodeReader::count_sections(""), 1); // max(0, 1)
    }

    #[test]
    fn test_project_name_from_slug() {
        assert_eq!(
            ClaudeCodeReader::project_name_from_slug("-Users-foo-myproject"),
            "myproject"
        );
        assert_eq!(
            ClaudeCodeReader::project_name_from_slug("-Users-ayoamadi-projects-memcp"),
            "memcp"
        );
    }

    #[test]
    fn test_split_into_sections_basic() {
        let content = "# Architecture\nRust project using Tokio.\n\n## Storage\nPostgres with pgvector.";
        let chunks = ClaudeCodeReader::split_into_sections(content, "test.md", None);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("Architecture"));
        assert!(chunks[1].content.contains("Storage"));
        assert_eq!(chunks[0].type_hint, Some("fact".to_string()));
    }

    #[test]
    fn test_split_into_sections_no_headers() {
        let content = "Just some content without headers. It should be imported as one chunk.";
        let chunks = ClaudeCodeReader::split_into_sections(content, "test.md", None);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_split_project_tag() {
        let chunks = ClaudeCodeReader::split_into_sections("# Test\nContent", "test.md", Some("myproject"));
        assert_eq!(chunks[0].project, Some("myproject".to_string()));
    }

    #[tokio::test]
    async fn test_read_memory_md_file() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(f, "# User Preferences\nDark mode preferred.\n## Editor\nVSCode with Rust analyzer.").unwrap();

        let reader = ClaudeCodeReader::new(false);
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(f.path(), &opts).await.unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("User Preferences"));
    }

    #[tokio::test]
    async fn test_read_empty_memory_md() {
        let f = NamedTempFile::with_suffix(".md").unwrap();
        let reader = ClaudeCodeReader::new(false);
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(f.path(), &opts).await.unwrap();
        assert_eq!(chunks.len(), 0);
    }

    #[tokio::test]
    async fn test_discover_no_claude_dir() {
        let reader = ClaudeCodeReader::new(false);
        // Just verify it doesn't panic when ~/.claude doesn't exist or has no MEMORY.md files
        // that we'd need to clean up.
        let result = reader.discover().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_read_history_jsonl_assistant_only() {
        let dir = TempDir::new().unwrap();
        let jsonl_path = dir.path().join("history.jsonl");
        let mut f = std::fs::File::create(&jsonl_path).unwrap();
        writeln!(f, r#"{{"type":"user","message":"Hello there"}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":"I've implemented the feature with proper error handling."}}"#).unwrap();
        writeln!(f, r#"{{"type":"system","message":"System prompt here"}}"#).unwrap();

        let chunks = ClaudeCodeReader::read_history_jsonl(&jsonl_path).unwrap();
        // Only assistant message should be imported
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].type_hint, Some("observation".to_string()));
        assert!(chunks[0].content.contains("implemented"));
    }

    #[test]
    fn test_extract_message_text_string() {
        let msg = Some(serde_json::Value::String("Hello world".to_string()));
        assert_eq!(extract_message_text(&msg), Some("Hello world".to_string()));
    }

    #[test]
    fn test_extract_message_text_object_content() {
        let msg = Some(serde_json::json!({
            "content": "This is the response text"
        }));
        assert_eq!(extract_message_text(&msg), Some("This is the response text".to_string()));
    }

    #[test]
    fn test_extract_message_text_content_array() {
        let msg = Some(serde_json::json!({
            "content": [
                {"type": "text", "text": "First part"},
                {"type": "tool_use", "id": "toolu_123"},
                {"type": "text", "text": "Second part"}
            ]
        }));
        let result = extract_message_text(&msg).unwrap();
        assert!(result.contains("First part"));
        assert!(result.contains("Second part"));
        assert!(!result.contains("tool_use"));
    }

    #[test]
    fn test_source_name() {
        let reader = ClaudeCodeReader::new(false);
        assert_eq!(reader.source_name(), "claude-code");
        assert_eq!(reader.source_kind(), ImportSourceKind::ClaudeCode);
    }
}
