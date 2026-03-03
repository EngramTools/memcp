//! Checkpoint and report for import pipeline.
//!
//! Checkpoints allow resuming interrupted imports.
//! Reports summarize import results after completion.
//!
//! Files are written to `~/.memcp/imports/<source>-<timestamp>/`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::ImportResult;

/// Checkpoint saved after each batch, enabling resume on interruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Source name (e.g., "jsonl", "openclaw").
    pub source: String,
    /// Path to the imported file/directory.
    pub path: String,
    /// Index of the last successfully completed batch (0-based).
    pub last_batch: usize,
    /// Total number of batches in this import.
    pub total_batches: usize,
    /// Timestamp when the checkpoint was saved.
    pub timestamp: DateTime<Utc>,
    /// Cumulative import result up to and including last_batch.
    pub result_so_far: ImportResult,
}

impl Checkpoint {
    /// Save checkpoint to `<dir>/checkpoint.json`.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join("checkpoint.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Load checkpoint from `<dir>/checkpoint.json`. Returns None if not found.
    pub fn load(dir: &Path) -> Option<Self> {
        let path = dir.join("checkpoint.json");
        if !path.exists() {
            return None;
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(cp) => Some(cp),
                Err(e) => {
                    warn!("Failed to parse checkpoint at {:?}: {}", path, e);
                    None
                }
            },
            Err(e) => {
                warn!("Failed to read checkpoint at {:?}: {}", path, e);
                None
            }
        }
    }
}

/// Final report written after a successful import run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReport {
    pub source: String,
    pub path: String,
    pub total: usize,
    pub imported: usize,
    pub filtered: usize,
    pub failed: usize,
    pub skipped_dedup: usize,
    pub errors: Vec<super::ImportError>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    /// Approximate duration in seconds.
    pub duration_secs: u64,
}

impl ImportReport {
    /// Write report to `<dir>/report.json`.
    pub fn write_report(&self, dir: &Path) -> Result<()> {
        let path = dir.join("report.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

/// Return the import directory path for a given source.
///
/// Format: `~/.memcp/imports/<source>-<timestamp>-<short-id>/`
/// Both timestamp and a random short ID ensure uniqueness per run.
pub fn import_dir(source: &str) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    // Use 6 random hex chars for uniqueness within the same second.
    let short_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let dir_name = format!("{}-{}-{}", source, timestamp, short_id);

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".memcp")
        .join("imports")
        .join(dir_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_checkpoint_save_load_roundtrip() {
        let dir = tempdir().unwrap();
        let cp = Checkpoint {
            source: "jsonl".to_string(),
            path: "/tmp/test.jsonl".to_string(),
            last_batch: 2,
            total_batches: 5,
            timestamp: Utc::now(),
            result_so_far: ImportResult {
                total: 300,
                imported: 100,
                filtered: 50,
                failed: 0,
                skipped_dedup: 10,
                errors: vec![],
            },
        };

        cp.save(dir.path()).unwrap();
        let loaded = Checkpoint::load(dir.path()).unwrap();

        assert_eq!(loaded.source, "jsonl");
        assert_eq!(loaded.last_batch, 2);
        assert_eq!(loaded.result_so_far.imported, 100);
    }

    #[test]
    fn test_checkpoint_load_missing_returns_none() {
        let dir = tempdir().unwrap();
        let loaded = Checkpoint::load(dir.path());
        assert!(loaded.is_none());
    }

    #[test]
    fn test_import_dir_contains_source_name() {
        let dir = import_dir("jsonl");
        let name = dir.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("jsonl-"));
    }
}
