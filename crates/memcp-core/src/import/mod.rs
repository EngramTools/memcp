//! Import pipeline — brings external AI tool history into memcp.
//!
//! Three-tier pipeline:
//! 1. Noise filter (rule-based, instant) — drops low-signal chunks
//! 2. Dedup (SHA-256 normalized hash) — prevents duplicate imports
//! 3. Batch INSERT (direct Postgres, 100 items/tx) — high-throughput insertion
//!
//! `ImportEngine::run()` drives the pipeline end-to-end, with progress bar,
//! checkpoint resume, and import report generation.

pub mod noise;
pub mod dedup;
pub mod batch;
pub mod checkpoint;
pub mod jsonl;
pub mod openclaw;
// claude_code, chatgpt, claude_ai, markdown readers in later plans

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::storage::store::postgres::PostgresMemoryStore;

/// A single chunk of content to be imported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportChunk {
    pub content: String,
    pub type_hint: Option<String>,
    pub source: String,
    pub tags: Vec<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub actor: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub workspace: Option<String>,
}

/// Options controlling import behavior.
#[derive(Debug, Clone)]
pub struct ImportOpts {
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub skip_embeddings: bool,
    pub batch_size: usize,
    pub since: Option<DateTime<Utc>>,
    pub dry_run: bool,
    pub curate: bool,
    pub skip_patterns: Vec<String>,
}

impl Default for ImportOpts {
    fn default() -> Self {
        Self {
            project: None,
            tags: vec![],
            skip_embeddings: false,
            batch_size: 100,
            since: None,
            dry_run: false,
            curate: false,
            skip_patterns: vec![],
        }
    }
}

/// Auto-detected importable source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSource {
    pub path: PathBuf,
    pub source_type: String,
    pub item_count: usize,
    pub description: String,
}

/// Summary of an import run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportResult {
    pub total: usize,
    pub imported: usize,
    pub filtered: usize,
    pub failed: usize,
    pub skipped_dedup: usize,
    pub errors: Vec<ImportError>,
}

/// An error for a single item during import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportError {
    pub content_preview: String,
    pub reason: String,
}

/// The kind of import source — used for noise pattern selection and tagging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportSourceKind {
    OpenClaw,
    ClaudeCode,
    ChatGpt,
    ClaudeAi,
    Markdown,
    Jsonl,
}

impl ImportSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenClaw => "openclaw",
            Self::ClaudeCode => "claude-code",
            Self::ChatGpt => "chatgpt",
            Self::ClaudeAi => "claude",
            Self::Markdown => "markdown",
            Self::Jsonl => "jsonl",
        }
    }
}

/// Trait implemented by each source reader (JSONL, OpenClaw, Claude Code, etc.).
#[async_trait]
pub trait ImportSource: Send + Sync {
    /// Short name used in tags and logging (e.g., "jsonl", "openclaw").
    fn source_name(&self) -> &str;

    /// Source kind — used to select noise patterns.
    fn source_kind(&self) -> ImportSourceKind;

    /// Hardcoded noise patterns for this source (e.g., heartbeat strings for OpenClaw).
    fn noise_patterns(&self) -> Vec<&'static str>;

    /// Auto-discover local instances of this source (e.g., scan ~/.openclaw/).
    /// Returns empty vec for sources without auto-discovery (e.g., JSONL).
    async fn discover(&self) -> Result<Vec<DiscoveredSource>>;

    /// Read all chunks from the given path, respecting opts.since filter.
    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>>;
}

/// Drives the three-tier import pipeline for a single source + path.
pub struct ImportEngine {
    store: Arc<PostgresMemoryStore>,
    /// Base noise filter (user-supplied patterns only). Source-specific patterns
    /// are merged in `run()` via `NoiseFilter::new_with_source_patterns()`.
    _noise_filter: noise::NoiseFilter,
    opts: ImportOpts,
}

impl ImportEngine {
    pub fn new(store: Arc<PostgresMemoryStore>, opts: ImportOpts) -> Self {
        let _noise_filter = noise::NoiseFilter::new(&opts.skip_patterns);
        Self { store, _noise_filter, opts }
    }

    /// Run the full import pipeline: read → filter → dedup → batch insert → checkpoint → report.
    pub async fn run(&self, source: &dyn ImportSource, path: &Path) -> Result<ImportResult> {
        let import_dir = checkpoint::import_dir(source.source_name());
        std::fs::create_dir_all(&import_dir)?;

        // Check for existing checkpoint to resume from.
        let existing_checkpoint = checkpoint::Checkpoint::load(&import_dir);
        let resume_from_batch = existing_checkpoint.as_ref().map(|c| c.last_batch + 1).unwrap_or(0);
        let mut result = existing_checkpoint
            .as_ref()
            .map(|c| c.result_so_far.clone())
            .unwrap_or_default();

        if resume_from_batch > 0 {
            info!(
                "Resuming import from batch {} (checkpoint found at {:?})",
                resume_from_batch, import_dir
            );
        }

        // Step 1: Read all chunks from source.
        info!("Reading chunks from {:?} via {}", path, source.source_name());
        let all_chunks = source.read_chunks(path, &self.opts).await?;
        result.total = all_chunks.len();

        // Step 2: Apply --since filter.
        let chunks: Vec<ImportChunk> = if let Some(since) = self.opts.since {
            all_chunks
                .into_iter()
                .filter(|c| c.created_at.map(|t| t >= since).unwrap_or(true))
                .collect()
        } else {
            all_chunks
        };

        // Add source-specific noise patterns from the source reader.
        let source_patterns: Vec<String> = source.noise_patterns().iter().map(|s| s.to_string()).collect();
        let noise_filter = noise::NoiseFilter::new_with_source_patterns(&self.opts.skip_patterns, &source_patterns);

        // Step 3: Noise filter.
        let mut survivors = Vec::new();
        let mut filtered_chunks = Vec::new();
        for chunk in chunks {
            if noise_filter.is_noise(&chunk.content) {
                filtered_chunks.push(chunk);
                result.filtered += 1;
            } else {
                survivors.push(chunk);
            }
        }

        // Dry-run: show what would be imported and return.
        if self.opts.dry_run {
            println!("Dry run — would import {} items ({} filtered by noise)", survivors.len(), result.filtered);
            for chunk in &survivors {
                let preview = chunk.content.chars().take(80).collect::<String>();
                println!("  [{}] {}", chunk.source, preview);
            }
            result.imported = survivors.len();
            return Ok(result);
        }

        // Step 4: Batch-level dedup (within this import) + store-level dedup.
        let pool = self.store.pool();
        let all_hashes: Vec<String> = survivors.iter()
            .map(|c| dedup::normalized_hash(&c.content))
            .collect();
        let existing_hashes = dedup::check_existing(pool, &all_hashes).await?;
        let mut batch_seen: HashSet<String> = HashSet::new();

        let mut deduped = Vec::new();
        for (chunk, hash) in survivors.into_iter().zip(all_hashes.iter()) {
            if batch_seen.contains(hash) || existing_hashes.contains(hash) {
                result.skipped_dedup += 1;
                continue;
            }
            batch_seen.insert(hash.clone());
            deduped.push(chunk);
        }

        // Step 5: Batch into groups.
        let batches: Vec<Vec<ImportChunk>> = deduped
            .chunks(self.opts.batch_size)
            .map(|b| b.to_vec())
            .collect();
        let total_batches = batches.len();

        // Progress bar.
        let pb = ProgressBar::new(deduped.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} | filtered: {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(result.filtered.to_string());

        // Step 6: Process each batch.
        for (batch_idx, batch) in batches.iter().enumerate() {
            // Skip already-completed batches if resuming.
            if batch_idx < resume_from_batch {
                pb.inc(batch.len() as u64);
                continue;
            }

            let batch_result = batch::batch_insert_memories(pool, batch, &self.opts).await;
            match batch_result {
                Ok(br) => {
                    result.imported += br.inserted;
                    result.skipped_dedup += br.skipped;
                    pb.inc(batch.len() as u64);
                }
                Err(e) => {
                    warn!("Batch {} failed: {}", batch_idx, e);
                    // Record each item in the batch as failed.
                    for chunk in batch {
                        result.failed += 1;
                        result.errors.push(ImportError {
                            content_preview: chunk.content.chars().take(100).collect(),
                            reason: e.to_string(),
                        });
                    }
                }
            }

            // Save checkpoint after each batch.
            let cp = checkpoint::Checkpoint {
                source: source.source_name().to_string(),
                path: path.to_string_lossy().to_string(),
                last_batch: batch_idx,
                total_batches,
                timestamp: Utc::now(),
                result_so_far: result.clone(),
            };
            if let Err(e) = cp.save(&import_dir) {
                warn!("Failed to save checkpoint: {}", e);
            }
        }

        pb.finish_with_message(format!("done (filtered: {})", result.filtered));

        // Write final report.
        let report = checkpoint::ImportReport {
            source: source.source_name().to_string(),
            path: path.to_string_lossy().to_string(),
            total: result.total,
            imported: result.imported,
            filtered: result.filtered,
            failed: result.failed,
            skipped_dedup: result.skipped_dedup,
            errors: result.errors.clone(),
            started_at: existing_checkpoint.as_ref().map(|c| c.timestamp).unwrap_or_else(Utc::now),
            completed_at: Utc::now(),
            duration_secs: 0, // approximate — actual timing would need start recording
        };
        if let Err(e) = report.write_report(&import_dir) {
            warn!("Failed to write import report: {}", e);
        }

        info!(
            "Import complete: {} imported, {} filtered, {} dedup-skipped, {} failed",
            result.imported, result.filtered, result.skipped_dedup, result.failed
        );

        Ok(result)
    }
}
