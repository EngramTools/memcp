//! JSONL reader — imports memories from JSONL files (round-trip format).
//!
//! Each line is a JSON object with fields matching the Memory struct.
//! This is the simplest reader and validates the full pipeline end-to-end.
//!
//! Format: one memory per line, all fields except `content` are optional.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Deserializable representation of a single JSONL memory line.
#[derive(Debug, Deserialize, Serialize)]
struct JsonlMemoryLine {
    content: String,
    type_hint: Option<String>,
    source: Option<String>,
    tags: Option<Vec<String>>,
    created_at: Option<String>,
    actor: Option<String>,
    workspace: Option<String>,
    // Embedding fields for zero-cost reuse (round-trip with --include-embeddings).
    embedding: Option<Vec<f32>>,
    embedding_model: Option<String>,
}

/// JSONL source reader. Implements ImportSource for .jsonl files.
pub struct JsonlReader;

#[async_trait]
impl ImportSource for JsonlReader {
    fn source_name(&self) -> &str {
        "jsonl"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::Jsonl
    }

    /// JSONL is pre-curated — no hardcoded noise patterns.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![]
    }

    /// No auto-discovery for generic JSONL files.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        Ok(vec![])
    }

    /// Read all chunks from the given JSONL file.
    ///
    /// Each line is parsed as a JSON object. Parse errors are collected as
    /// ImportErrors and processing continues (fail-open per design).
    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open JSONL file {:?}: {}", path, e))?;
        let reader = BufReader::new(file);

        let mut chunks = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!("Failed to read line {}: {}", line_num + 1, e);
                    continue;
                }
            };

            let trimmed = line.trim();
            // Skip blank lines and comment lines.
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            let parsed: JsonlMemoryLine = match serde_json::from_str(trimmed) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Line {}: JSON parse error: {}", line_num + 1, e);
                    continue;
                }
            };

            // Parse created_at from ISO 8601 string.
            let created_at = parsed.created_at.as_deref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            });

            // Apply opts.since filter.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            let source = parsed.source
                .unwrap_or_else(|| "imported:jsonl".to_string());

            let chunk = ImportChunk {
                content: parsed.content,
                type_hint: parsed.type_hint,
                source,
                tags: parsed.tags.unwrap_or_default(),
                created_at,
                actor: parsed.actor,
                embedding: parsed.embedding,
                embedding_model: parsed.embedding_model,
                workspace: parsed.workspace,
            };

            chunks.push(chunk);
        }

        Ok(chunks)
    }
}

/// Discover JSONL files in a directory (helper for tests and --discover).
pub fn discover_jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_jsonl_file(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::with_suffix(".jsonl").unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[tokio::test]
    async fn test_read_valid_jsonl() {
        let file = make_jsonl_file(&[
            r#"{"content":"User prefers Rust for backend services due to memory safety"}"#,
            r#"{"content":"Dark mode is preferred for coding sessions","type_hint":"preference","tags":["ui","editor"]}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].type_hint, None);
        assert_eq!(chunks[1].type_hint, Some("preference".to_string()));
        assert_eq!(chunks[1].tags, vec!["ui".to_string(), "editor".to_string()]);
    }

    #[tokio::test]
    async fn test_skips_blank_lines() {
        let file = make_jsonl_file(&[
            r#"{"content":"First memory that should be long enough to pass noise filter"}"#,
            "",
            r#"{"content":"Second memory that should also pass the noise filter check"}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        assert_eq!(chunks.len(), 2);
    }

    #[tokio::test]
    async fn test_skips_invalid_json_lines() {
        let file = make_jsonl_file(&[
            r#"{"content":"Valid memory line with sufficient content to pass"}"#,
            r#"not valid json at all"#,
            r#"{"content":"Another valid memory line with enough content here"}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        // Invalid line is skipped, valid lines are returned.
        assert_eq!(chunks.len(), 2);
    }

    #[tokio::test]
    async fn test_since_filter() {
        let file = make_jsonl_file(&[
            r#"{"content":"Old memory from before the filter cutoff date","created_at":"2020-01-01T00:00:00Z"}"#,
            r#"{"content":"New memory from after the filter cutoff date here","created_at":"2025-01-01T00:00:00Z"}"#,
        ]);

        let reader = JsonlReader;
        let mut opts = ImportOpts::default();
        opts.since = Some(DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z").unwrap().with_timezone(&chrono::Utc));

        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("New"));
    }

    #[tokio::test]
    async fn test_source_name() {
        let reader = JsonlReader;
        assert_eq!(reader.source_name(), "jsonl");
        assert_eq!(reader.source_kind(), ImportSourceKind::Jsonl);
    }
}
