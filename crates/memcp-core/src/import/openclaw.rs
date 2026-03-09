//! OpenClaw reader — imports memory chunks from OpenClaw SQLite databases.
//!
//! OpenClaw stores memories in per-agent SQLite databases at `~/.openclaw/memory/*.sqlite`.
//! Each database has a `chunks` table with columns:
//!   id, path, source, text, embedding, model, updated_at
//!
//! Source classification:
//!   - source="memory" → type_hint="fact" (high-signal, curated memory chunks)
//!   - source="sessions" → type_hint="observation" (conversation history)
//!
//! Embedding reuse: if the stored model name and vector dimension match the
//! configured memcp embedding model, the existing vector is reused (zero API cost).
//! Otherwise, chunk is stored with embedding_status=pending for async re-embedding.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
use tracing::{debug, warn};

use crate::config::EmbeddingConfig;
use crate::embedding::model_dimension;

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Row from OpenClaw chunks table.
#[derive(Debug)]
struct ChunkRow {
    _id: String,
    text: String,
    source: String,
    path: String,
    embedding: Option<String>,
    model: Option<String>,
    /// updated_at as milliseconds since Unix epoch (INTEGER in OpenClaw SQLite).
    updated_at_ms: Option<i64>,
}

/// Reads memories from OpenClaw SQLite databases.
pub struct OpenClawReader {
    /// Filter to a specific agent name (from --agent flag).
    agent_filter: Option<String>,
    /// Configured embedding model name for reuse comparison.
    configured_model: Option<String>,
    /// Configured embedding dimension for reuse comparison.
    configured_dimension: Option<usize>,
}

impl OpenClawReader {
    /// Create a new OpenClawReader.
    ///
    /// Reads the configured model and dimension from `embedding_config` to
    /// enable embedding reuse when model+dimension match stored OpenClaw vectors.
    pub fn new(agent: Option<String>, embedding_config: &EmbeddingConfig) -> Self {
        let configured_model = if embedding_config.provider == "openai" {
            Some(embedding_config.openai_model.clone())
        } else {
            Some(embedding_config.local_model.clone())
        };

        let configured_dimension = embedding_config.dimension.or_else(|| {
            configured_model.as_deref().and_then(model_dimension)
        });

        Self {
            agent_filter: agent,
            configured_model,
            configured_dimension,
        }
    }

    /// Extract the agent name from an OpenClaw chunk path.
    ///
    /// Path examples:
    ///   "memory/2026-01-15/daily-log"     → "default"
    ///   "sessions/vita/2026-01-15/chat"   → "vita"
    fn extract_agent(path: &str, source: &str, file_stem: &str) -> String {
        if source == "sessions" {
            // sessions/{agent}/{date}/... — agent is the second segment
            let parts: Vec<&str> = path.splitn(4, '/').collect();
            if parts.len() >= 2 {
                let candidate = parts[1];
                // Filter out date-like segments (YYYY-MM-DD)
                if !candidate.contains('-') || candidate.len() != 10 {
                    return candidate.to_string();
                }
            }
        }
        // For memory source or fallback: use the SQLite filename stem
        // (e.g., "vita" from "~/.openclaw/memory/vita.sqlite")
        file_stem.to_string()
    }

}

#[async_trait]
impl ImportSource for OpenClawReader {
    fn source_name(&self) -> &str {
        "openclaw"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::OpenClaw
    }

    /// OpenClaw-specific noise patterns — operational signals, not memories.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![
            "HEARTBEAT_OK",
            "Token Monitor Report",
            "Switchboard - Cross-Subagent",
            "FailoverError: LLM request timed out",
            "Exec failed",
            "Exec completed",
            "compinit: initialization aborted",
        ]
    }

    /// Scan `~/.openclaw/memory/` for SQLite databases.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => {
                warn!("Could not determine home directory for OpenClaw discovery");
                return Ok(vec![]);
            }
        };

        let openclaw_dir = home.join(".openclaw").join("memory");
        if !openclaw_dir.exists() {
            debug!("OpenClaw memory dir not found at {:?}", openclaw_dir);
            return Ok(vec![]);
        }

        let mut sources = Vec::new();

        let entries = match std::fs::read_dir(&openclaw_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read OpenClaw directory {:?}: {}", openclaw_dir, e);
                return Ok(vec![]);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "sqlite").unwrap_or(false) {
                let file_stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".to_string());

                // Count rows in the chunks table.
                let count = match count_chunks(&path) {
                    Ok(n) => n,
                    Err(e) => {
                        warn!("Could not count chunks in {:?}: {}", path, e);
                        continue;
                    }
                };

                sources.push(DiscoveredSource {
                    path,
                    source_type: "openclaw".to_string(),
                    item_count: count,
                    description: format!("OpenClaw agent {}: {} chunks", file_stem, count),
                });
            }
        }

        // Sort by path for deterministic order.
        sources.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(sources)
    }

    /// Read all chunks from the given OpenClaw SQLite file.
    ///
    /// All rusqlite calls run inside `tokio::task::spawn_blocking` to avoid
    /// blocking the async executor (Pitfall 1 from RESEARCH.md).
    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let path = path.to_path_buf();
        let agent_filter = self.agent_filter.clone();
        let configured_model = self.configured_model.clone();
        let configured_dimension = self.configured_dimension;

        // Extract file stem for agent name fallback.
        let file_stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());

        let since = opts.since;

        let chunks = tokio::task::spawn_blocking(move || -> Result<Vec<ImportChunk>> {
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("Failed to open OpenClaw database {:?}", path))?;

            let mut sql = String::from(
                "SELECT id, text, source, path, embedding, model, updated_at FROM chunks"
            );

            // Apply agent filter via path LIKE clause.
            if let Some(ref agent) = agent_filter {
                sql.push_str(&format!(
                    " WHERE path LIKE '%/{agent}/%' OR path LIKE '%/{agent}'",
                    agent = agent
                ));
            }

            let mut stmt = conn.prepare(&sql)
                .with_context(|| "Failed to prepare OpenClaw chunks query")?;

            let rows: Vec<ChunkRow> = stmt.query_map([], |row| {
                Ok(ChunkRow {
                    _id: row.get(0)?,
                    text: row.get(1)?,
                    source: row.get(2)?,
                    path: row.get(3)?,
                    embedding: row.get(4)?,
                    model: row.get(5)?,
                    // updated_at is INTEGER (ms since epoch) in OpenClaw SQLite.
                    updated_at_ms: row.get(6).ok(),
                })
            })
            .with_context(|| "Failed to query OpenClaw chunks")?
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!("Skipping malformed OpenClaw row: {}", e);
                    None
                }
            })
            .collect();

            let mut import_chunks = Vec::with_capacity(rows.len());

            for row in rows {
                // Parse created_at from updated_at_ms (milliseconds since Unix epoch).
                let created_at = row.updated_at_ms.and_then(|ms| {
                    let secs = ms / 1000;
                    let nsecs = ((ms % 1000) * 1_000_000) as u32;
                    DateTime::from_timestamp(secs, nsecs)
                });

                // Apply --since filter.
                if let (Some(since_dt), Some(chunk_dt)) = (since, created_at) {
                    if chunk_dt < since_dt {
                        continue;
                    }
                }

                // Map source column to type_hint.
                let type_hint = match row.source.as_str() {
                    "memory" => Some("fact".to_string()),
                    "sessions" => Some("observation".to_string()),
                    other => {
                        debug!("Unknown OpenClaw source type '{}', defaulting to observation", other);
                        Some("observation".to_string())
                    }
                };

                // Extract agent name from path.
                let agent_name = Self::extract_agent(&row.path, &row.source, &file_stem);

                // Build tags.
                let tags = vec![
                    "imported".to_string(),
                    "imported:openclaw".to_string(),
                    format!("agent:{}", agent_name),
                ];

                // Attempt embedding reuse if model+dimension match.
                let (embedding, embedding_model) = if let (Some(emb_json), Some(model)) =
                    (row.embedding.as_deref(), row.model.as_deref())
                {
                    let reuse = try_reuse_embedding_inner(
                        emb_json,
                        model,
                        configured_model.as_deref(),
                        configured_dimension,
                    );
                    if reuse.is_some() {
                        (reuse, Some(model.to_string()))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                import_chunks.push(ImportChunk {
                    content: row.text,
                    type_hint,
                    source: "imported:openclaw".to_string(),
                    tags,
                    created_at,
                    actor: Some(agent_name),
                    embedding,
                    embedding_model,
                    project: None,
                });
            }

            Ok(import_chunks)
        })
        .await
        .with_context(|| "spawn_blocking task panicked")??;

        Ok(chunks)
    }
}

/// Count chunks in an OpenClaw SQLite file (read-only, synchronous helper).
fn count_chunks(path: &PathBuf) -> Result<usize> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
    Ok(count as usize)
}

/// Pure function for embedding reuse check — called inside spawn_blocking closure.
fn try_reuse_embedding_inner(
    embedding_json: &str,
    model: &str,
    configured_model: Option<&str>,
    configured_dimension: Option<usize>,
) -> Option<Vec<f32>> {
    let configured_model = configured_model?;
    let configured_dim = configured_dimension?;

    // Model name must match (case-insensitive).
    if !model.eq_ignore_ascii_case(configured_model) {
        return None;
    }

    // Parse the embedding JSON.
    let embedding: Vec<f32> = serde_json::from_str(embedding_json).ok()?;

    // Dimension must match.
    if embedding.len() != configured_dim {
        return None;
    }

    Some(embedding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EmbeddingConfig;

    fn default_config() -> EmbeddingConfig {
        EmbeddingConfig {
            provider: "local".to_string(),
            openai_api_key: None,
            cache_dir: "/tmp/memcp_models".to_string(),
            local_model: "AllMiniLML6V2".to_string(),
            openai_model: "text-embedding-3-small".to_string(),
            openai_base_url: None,
            dimension: Some(384),
            reembed_on_tag_change: false,
            tiers: Default::default(),
        }
    }

    #[test]
    fn test_extract_agent_sessions() {
        assert_eq!(
            OpenClawReader::extract_agent("sessions/vita/2026-01-15/chat", "sessions", "default"),
            "vita"
        );
    }

    #[test]
    fn test_extract_agent_memory_fallback() {
        assert_eq!(
            OpenClawReader::extract_agent("memory/2026-01-15/daily-log", "memory", "vita"),
            "vita"
        );
    }

    #[test]
    fn test_extract_agent_sessions_no_date_segment() {
        assert_eq!(
            OpenClawReader::extract_agent("sessions/myagent", "sessions", "default"),
            "myagent"
        );
    }

    #[test]
    fn test_embedding_reuse_match() {
        let v: Vec<f32> = vec![0.1; 384];
        let json = serde_json::to_string(&v).unwrap();
        let result = try_reuse_embedding_inner(&json, "AllMiniLML6V2", Some("AllMiniLML6V2"), Some(384));
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 384);
    }

    #[test]
    fn test_embedding_reuse_model_mismatch() {
        let v: Vec<f32> = vec![0.1; 384];
        let json = serde_json::to_string(&v).unwrap();
        let result = try_reuse_embedding_inner(&json, "OtherModel", Some("AllMiniLML6V2"), Some(384));
        assert!(result.is_none());
    }

    #[test]
    fn test_embedding_reuse_dimension_mismatch() {
        let v: Vec<f32> = vec![0.1; 512];
        let json = serde_json::to_string(&v).unwrap();
        let result = try_reuse_embedding_inner(&json, "AllMiniLML6V2", Some("AllMiniLML6V2"), Some(384));
        assert!(result.is_none());
    }

    #[test]
    fn test_openclaw_reader_new() {
        let config = default_config();
        let reader = OpenClawReader::new(Some("vita".to_string()), &config);
        assert_eq!(reader.agent_filter, Some("vita".to_string()));
        assert_eq!(reader.configured_dimension, Some(384));
    }

    #[test]
    fn test_noise_patterns() {
        let config = default_config();
        let reader = OpenClawReader::new(None, &config);
        let patterns = reader.noise_patterns();
        assert!(patterns.contains(&"HEARTBEAT_OK"));
        assert!(patterns.contains(&"Token Monitor Report"));
        assert!(patterns.len() >= 5);
    }

    #[tokio::test]
    async fn test_discover_no_openclaw_dir() {
        // When ~/.openclaw/memory doesn't exist, discover returns empty vec (no error).
        // We can't easily mock home dir, so just verify it runs without panic.
        let config = default_config();
        let reader = OpenClawReader::new(None, &config);
        let result = reader.discover().await;
        assert!(result.is_ok());
    }
}
