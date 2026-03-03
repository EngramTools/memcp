//! Import history tracking — prevents accidental re-imports.
//!
//! Stores a JSONL log at `~/.memcp/imports/history.jsonl`. Each line is a
//! serialized `ImportHistoryRecord`. `find_record` returns the most recent
//! entry for a given source type.
//!
//! Path-specific variants (`load_history_from`, `find_record_in`,
//! `append_record_to`) are exported for use in tests with custom paths.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;

/// One entry in the import history log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportHistoryRecord {
    /// The source system (e.g., "openclaw", "claude-code", "chatgpt").
    pub source_type: String,
    /// The file path that was imported.
    pub path: String,
    /// Number of memories successfully imported in this run.
    pub count: usize,
    /// When this import completed.
    pub timestamp: DateTime<Utc>,
}

/// Return the canonical history file path.
///
/// Uses `~/.memcp/imports/history.jsonl` in production.
pub fn default_history_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".memcp")
        .join("imports")
        .join("history.jsonl")
}

// ---------------------------------------------------------------------------
// Path-specific helpers (used by tests and the public API)
// ---------------------------------------------------------------------------

/// Load all records from the given JSONL file.
///
/// A missing file is silently treated as empty history.
pub fn load_history_from(path: &Path) -> Vec<ImportHistoryRecord> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines().map_while(|l| l.ok()) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<ImportHistoryRecord>(trimmed) {
            records.push(record);
        }
    }
    records
}

/// Return the most recent record for `source_type` in the given file, or `None`.
pub fn find_record_in(path: &Path, source_type: &str) -> Option<ImportHistoryRecord> {
    load_history_from(path)
        .into_iter()
        .filter(|r| r.source_type == source_type)
        .max_by_key(|r| r.timestamp)
}

/// Append a record to the given JSONL file, creating it (and parents) if needed.
pub fn append_record_to(path: &Path, record: &ImportHistoryRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(record)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Convenience wrappers that use the default production path
// ---------------------------------------------------------------------------

/// Load all history records from the default path.
pub fn load_history() -> Vec<ImportHistoryRecord> {
    load_history_from(&default_history_path())
}

/// Return the most recent record for `source_type` from the default path.
pub fn find_record(source_type: &str) -> Option<ImportHistoryRecord> {
    find_record_in(&default_history_path(), source_type)
}

/// Append a record to the default history file.
pub fn append_record(record: &ImportHistoryRecord) -> Result<()> {
    append_record_to(&default_history_path(), record)
}
