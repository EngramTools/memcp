//! Markdown reader — imports .md files chunked by section headers.
//!
//! Handles both single files and directories of .md files.
//! Splits content at `# ` and `## ` header boundaries to produce
//! semantically coherent chunks (one section = one memory).
//!
//! Long sections are further split by `chunk_content()` to stay under 2048 chars.

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::{
    chatgpt::chunk_content, DiscoveredSource, ImportChunk, ImportOpts, ImportSource,
    ImportSourceKind,
};

/// Maximum chunk size in characters. Sections longer than this are further split.
const MAX_CHUNK_CHARS: usize = 2048;

// ── Reader ────────────────────────────────────────────────────────────────────

/// Markdown directory/file reader. Implements `ImportSource` for `.md` files.
pub struct MarkdownReader;

#[async_trait]
impl ImportSource for MarkdownReader {
    fn source_name(&self) -> &str {
        "markdown"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::Markdown
    }

    /// Markdown files are intentional content — no noise patterns.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![]
    }

    /// No auto-discovery for generic markdown files.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        Ok(vec![])
    }

    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        // Collect all .md files to process.
        let md_files = if path.is_dir() {
            collect_md_files(path)
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            vec![path.to_path_buf()]
        } else {
            // Try to open regardless — let the user decide what to import.
            vec![path.to_path_buf()]
        };

        let mut chunks = Vec::new();

        for md_path in md_files {
            let content = match std::fs::read_to_string(&md_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read {:?}: {}", md_path, e);
                    continue;
                }
            };

            // Get file modification time for created_at.
            let created_at: Option<DateTime<Utc>> = std::fs::metadata(&md_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::from);

            // Apply --since filter.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            let filename = md_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown.md")
                .to_string();

            let base_tags = vec![
                "imported".to_string(),
                "imported:markdown".to_string(),
                format!("file:{}", filename),
            ];

            // Split content at H1/H2 headers.
            let sections = split_by_headers(&content);

            for section in sections {
                let section = section.trim();
                if section.is_empty() {
                    continue;
                }

                // Further split very long sections.
                let sub_chunks = chunk_content(section, MAX_CHUNK_CHARS);
                let total = sub_chunks.len();

                for (i, piece) in sub_chunks.into_iter().enumerate() {
                    if piece.trim().is_empty() {
                        continue;
                    }
                    let mut chunk_tags = base_tags.clone();
                    if total > 1 {
                        chunk_tags.push(format!("chunk:{}/{}", i + 1, total));
                    }
                    chunks.push(ImportChunk {
                        content: piece,
                        type_hint: Some("fact".to_string()),
                        source: "imported:markdown".to_string(),
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

/// Recursively collect all `.md` files under a directory.
pub fn collect_md_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    collect_md_recursive(dir, &mut files);
    files.sort();
    files
}

fn collect_md_recursive(dir: &Path, acc: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_recursive(&path, acc);
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            acc.push(path);
        }
    }
}

/// Split markdown content at H1 (`# `) and H2 (`## `) boundaries.
///
/// The header line is included at the top of each section chunk so that
/// sections remain self-contained and searchable.
pub fn split_by_headers(content: &str) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        if (line.starts_with("# ") || line.starts_with("## ")) && !current.trim().is_empty() {
            sections.push(current.trim_end().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.trim().is_empty() {
        sections.push(current.trim_end().to_string());
    }

    sections
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn test_source_name() {
        let reader = MarkdownReader;
        assert_eq!(reader.source_name(), "markdown");
        assert_eq!(reader.source_kind(), ImportSourceKind::Markdown);
    }

    #[test]
    fn test_split_by_headers_no_headers() {
        let content = "Just some plain text.\nNo headers here.";
        let sections = split_by_headers(content);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].contains("plain text"));
    }

    #[test]
    fn test_split_by_headers_h1() {
        let content = "# Section One\nContent of section one.\n# Section Two\nContent two.";
        let sections = split_by_headers(content);
        assert_eq!(sections.len(), 2);
        assert!(sections[0].contains("Section One"));
        assert!(sections[1].contains("Section Two"));
    }

    #[test]
    fn test_split_by_headers_h2() {
        let content = "## First\nText A\n## Second\nText B";
        let sections = split_by_headers(content);
        assert_eq!(sections.len(), 2);
        assert!(sections[0].starts_with("## First"));
    }

    #[test]
    fn test_split_empty_content() {
        let sections = split_by_headers("");
        assert!(sections.is_empty());
    }

    #[tokio::test]
    async fn test_read_single_md_file() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(f, "# Memory Design\nUse salience-based recall for efficiency.\n\n## Implementation\nFNV-1a hash for dedup.").unwrap();

        let reader = MarkdownReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(f.path(), &opts).await.unwrap();

        assert!(chunks.len() >= 1);
        for chunk in &chunks {
            assert_eq!(chunk.type_hint, Some("fact".to_string()));
            assert!(chunk.tags.contains(&"imported:markdown".to_string()));
        }
    }

    #[tokio::test]
    async fn test_read_directory_of_md_files() {
        let dir = tempdir().unwrap();
        let f1 = dir.path().join("notes.md");
        let f2 = dir.path().join("decisions.md");
        std::fs::write(&f1, "# Note\nThis is a note with enough content to pass the noise filter.").unwrap();
        std::fs::write(&f2, "# Decision\nWe chose Rust for memory safety guarantees and performance.").unwrap();

        let reader = MarkdownReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(dir.path(), &opts).await.unwrap();

        assert_eq!(chunks.len(), 2);
        // Both files imported.
        let all_tags: Vec<_> = chunks.iter().flat_map(|c| c.tags.iter()).collect();
        let has_notes = all_tags.iter().any(|t| t.contains("notes.md"));
        let has_decisions = all_tags.iter().any(|t| t.contains("decisions.md"));
        assert!(has_notes, "notes.md not found in tags");
        assert!(has_decisions, "decisions.md not found in tags");
    }
}
