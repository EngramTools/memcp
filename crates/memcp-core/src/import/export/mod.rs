//! Export pipeline — extracts memories from memcp to external formats.
//!
//! Supports three output formats:
//! - JSONL: one memory per line, round-trip compatible with `memcp import jsonl`
//! - CSV: flat tabular format for spreadsheet analysis
//! - Markdown: human-readable archive grouped by type_hint
//!
//! `ExportEngine::run()` queries memories with optional filters and dispatches
//! to the appropriate formatter.

pub mod csv;
pub mod jsonl;
pub mod markdown;

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::info;

use crate::storage::store::postgres::PostgresMemoryStore;

/// Output format for the export command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFormat {
    /// JSONL — one JSON object per line (round-trip compatible with import)
    Jsonl,
    /// CSV — flat tabular format with headers
    Csv,
    /// Markdown — human-readable archive grouped by type_hint
    Markdown,
}

impl ExportFormat {
    /// Parse a format string (case-insensitive) into an ExportFormat.
    #[allow(clippy::should_implement_trait)] // Custom from_str with anyhow::bail!, not std::str::FromStr
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "jsonl" => Ok(Self::Jsonl),
            "csv" => Ok(Self::Csv),
            "markdown" | "md" => Ok(Self::Markdown),
            other => anyhow::bail!(
                "Unknown export format '{}'. Supported formats: jsonl, csv, markdown",
                other
            ),
        }
    }
}

/// Options controlling export behavior.
#[derive(Debug, Clone)]
pub struct ExportOpts {
    /// Output format — defaults to JSONL.
    pub format: ExportFormat,
    /// Output file path. None means write to stdout.
    pub output: Option<PathBuf>,
    /// Filter by project (NULL for global memories).
    pub project: Option<String>,
    /// Filter by tags — memories must have ALL specified tags.
    pub tags: Option<Vec<String>>,
    /// Filter by creation date — only export memories created on or after this time.
    pub since: Option<DateTime<Utc>>,
    /// Include embedding vectors in JSONL output (requires is_current=true embedding).
    pub include_embeddings: bool,
    /// Include FSRS/salience state in output.
    pub include_state: bool,
    /// Include knowledge graph entities, relationships, and facts in JSONL output.
    /// Only affects JSONL format; ignored for CSV and Markdown.
    pub include_graph: bool,
}

impl Default for ExportOpts {
    fn default() -> Self {
        Self {
            format: ExportFormat::Jsonl,
            output: None,
            project: None,
            tags: None,
            since: None,
            include_embeddings: false,
            include_state: false,
            include_graph: false,
        }
    }
}

/// A memory combined with optional salience state and embedding vector.
///
/// Each formatter receives a slice of ExportableMemory instances.
/// Fields that are not requested (e.g., embedding when include_embeddings=false)
/// are None and omitted from output.
#[derive(Debug, Clone)]
pub struct ExportableMemory {
    // Core memory fields.
    pub id: String,
    pub content: String,
    pub type_hint: String,
    pub source: String,
    pub tags: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub actor: Option<String>,
    pub actor_type: String,
    pub audience: String,
    pub project: Option<String>,
    pub event_time: Option<DateTime<Utc>>,
    pub event_time_precision: Option<String>,

    // Salience/FSRS state (populated when include_state=true).
    pub stability: Option<f64>,
    pub difficulty: Option<f64>,
    pub reinforcement_count: Option<i64>,
    pub last_reinforced_at: Option<DateTime<Utc>>,

    // Embedding vector (populated when include_embeddings=true).
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
}

/// A knowledge-graph entity record for export.
#[derive(Debug, Clone)]
pub struct ExportableEntity {
    pub name: String,
    pub entity_type: String,
    pub aliases: Value,
    pub metadata: Value,
}

/// A knowledge-graph relationship record for export.
/// Subject and object are referenced by (name, type) rather than UUID so the
/// export is portable across deployments.
#[derive(Debug, Clone)]
pub struct ExportableRelationship {
    pub subject_name: String,
    pub subject_type: String,
    pub object_name: String,
    pub object_type: String,
    pub predicate: String,
    pub relationship_type: String,
    pub weight: f64,
    pub confidence: f64,
}

/// A knowledge-graph fact record for export.
#[derive(Debug, Clone)]
pub struct ExportableFact {
    pub entity_name: String,
    pub entity_type: String,
    pub attribute: String,
    pub value: Value,
    pub confidence: f64,
}

/// Bundled graph records produced by `ExportEngine::fetch_graph`.
#[derive(Debug, Default)]
pub struct ExportableGraph {
    pub entities: Vec<ExportableEntity>,
    pub relationships: Vec<ExportableRelationship>,
    pub facts: Vec<ExportableFact>,
}

/// Export engine — queries memories from Postgres and dispatches to formatters.
pub struct ExportEngine {
    store: Arc<PostgresMemoryStore>,
}

impl ExportEngine {
    /// Create a new ExportEngine backed by the given Postgres store.
    pub fn new(store: Arc<PostgresMemoryStore>) -> Self {
        Self { store }
    }

    /// Run the export pipeline:
    /// 1. Query memories from DB with optional filters.
    /// 2. Optionally join salience state and embedding vectors.
    /// 3. Optionally fetch graph entities, relationships, and facts.
    /// 4. Open output writer (file or stdout).
    /// 5. Dispatch to format-specific formatter.
    ///
    /// Returns the number of memories exported.
    pub async fn run(&self, opts: &ExportOpts) -> Result<usize> {
        let pool = self.store.pool();

        let memories = self.fetch_memories(pool, opts).await?;
        let count = memories.len();

        let graph = if opts.include_graph && opts.format == ExportFormat::Jsonl {
            self.fetch_graph(pool).await?
        } else {
            ExportableGraph::default()
        };

        info!(count = count, format = ?opts.format, "Exporting memories");

        // Open output writer and dispatch to formatter.
        if let Some(ref path) = opts.output {
            let file = std::fs::File::create(path)
                .map_err(|e| anyhow::anyhow!("Failed to create output file {:?}: {}", path, e))?;
            let mut writer = BufWriter::new(file);
            match opts.format {
                ExportFormat::Jsonl => jsonl::write_jsonl(&mut writer, &memories, &graph, opts)?,
                ExportFormat::Csv => csv::write_csv(&mut writer, &memories, opts)?,
                ExportFormat::Markdown => markdown::write_markdown(&mut writer, &memories, opts)?,
            }
            writer.flush()?;
        } else {
            let stdout = io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            match opts.format {
                ExportFormat::Jsonl => jsonl::write_jsonl(&mut writer, &memories, &graph, opts)?,
                ExportFormat::Csv => csv::write_csv(&mut writer, &memories, opts)?,
                ExportFormat::Markdown => markdown::write_markdown(&mut writer, &memories, opts)?,
            }
            writer.flush()?;
        }

        Ok(count)
    }

    /// Run the export pipeline writing to an arbitrary writer.
    ///
    /// Used by the HTTP export endpoint to write directly to a response buffer.
    /// Returns the number of memories exported.
    pub async fn run_to_writer<W: std::io::Write>(
        &self,
        writer: &mut W,
        opts: &ExportOpts,
    ) -> Result<usize> {
        let pool = self.store.pool();
        let memories = self.fetch_memories(pool, opts).await?;
        let count = memories.len();

        let graph = if opts.include_graph && opts.format == ExportFormat::Jsonl {
            self.fetch_graph(pool).await?
        } else {
            ExportableGraph::default()
        };

        info!(count = count, format = ?opts.format, "Exporting memories to writer");

        match opts.format {
            ExportFormat::Jsonl => jsonl::write_jsonl(writer, &memories, &graph, opts)?,
            ExportFormat::Csv => csv::write_csv(writer, &memories, opts)?,
            ExportFormat::Markdown => markdown::write_markdown(writer, &memories, opts)?,
        }

        Ok(count)
    }

    /// Query memories from Postgres with optional filters.
    async fn fetch_memories(
        &self,
        pool: &sqlx::PgPool,
        opts: &ExportOpts,
    ) -> Result<Vec<ExportableMemory>> {
        // Build WHERE conditions dynamically.
        let mut conditions = vec!["m.deleted_at IS NULL".to_string()];
        let mut param_idx = 1usize;
        let mut params_project: Option<String> = None;
        let mut params_since: Option<DateTime<Utc>> = None;

        if opts.project.is_some() {
            conditions.push(format!("m.project = ${}", param_idx));
            params_project = opts.project.clone();
            param_idx += 1;
        }

        if opts.since.is_some() {
            conditions.push(format!("m.created_at >= ${}", param_idx));
            params_since = opts.since;
            param_idx += 1;
        }

        // Tags filter: each tag must appear in the JSONB tags array.
        // Each tag gets its own $N parameter to avoid SQL injection.
        let mut tag_params: Vec<serde_json::Value> = Vec::new();
        if let Some(ref tags) = opts.tags {
            for tag in tags {
                conditions.push(format!("m.tags @> ${}::jsonb", param_idx));
                tag_params.push(serde_json::json!([tag]));
                param_idx += 1;
            }
        }

        let where_clause = conditions.join(" AND ");

        // Choose between embedding join and non-embedding query.
        let sql = if opts.include_embeddings {
            format!(
                r#"
                SELECT
                    m.id, m.content, m.type_hint, m.source, m.tags,
                    m.created_at, m.actor, m.actor_type, m.audience,
                    m.project, m.event_time, m.event_time_precision,
                    ms.stability::float8, ms.difficulty::float8, ms.reinforcement_count, ms.last_reinforced_at,
                    me.embedding::text AS embedding_text, me.model_name AS embedding_model
                FROM memories m
                LEFT JOIN memory_salience ms ON m.id = ms.memory_id
                LEFT JOIN memory_embeddings me ON m.id = me.memory_id AND me.is_current = true
                WHERE {}
                ORDER BY m.created_at ASC
                "#,
                where_clause
            )
        } else {
            format!(
                r#"
                SELECT
                    m.id, m.content, m.type_hint, m.source, m.tags,
                    m.created_at, m.actor, m.actor_type, m.audience,
                    m.project, m.event_time, m.event_time_precision,
                    ms.stability::float8, ms.difficulty::float8, ms.reinforcement_count, ms.last_reinforced_at
                FROM memories m
                LEFT JOIN memory_salience ms ON m.id = ms.memory_id
                WHERE {}
                ORDER BY m.created_at ASC
                "#,
                where_clause
            )
        };

        // Execute query with dynamic parameters.
        let memories = if opts.include_embeddings {
            self.execute_query_with_embeddings(pool, &sql, params_project, params_since, tag_params)
                .await?
        } else {
            self.execute_query_no_embeddings(pool, &sql, params_project, params_since, tag_params)
                .await?
        };

        Ok(memories)
    }

    /// Fetch all active graph records (entities, relationships, facts) for export.
    ///
    /// Relationships and facts reference entities by (name, entity_type) rather than
    /// UUID so the output is portable. Rows are joined to the entities table to resolve
    /// names for subject/object/entity columns.
    async fn fetch_graph(&self, pool: &sqlx::PgPool) -> Result<ExportableGraph> {
        use sqlx::Row;

        // Entities — all rows (no soft-delete on entities).
        let entity_rows = sqlx::query(
            "SELECT name, entity_type, aliases, metadata FROM entities ORDER BY name",
        )
        .fetch_all(pool)
        .await?;

        let mut entities = Vec::with_capacity(entity_rows.len());
        for row in entity_rows {
            entities.push(ExportableEntity {
                name: row.try_get("name")?,
                entity_type: row.try_get("entity_type")?,
                aliases: row.try_get("aliases")?,
                metadata: row.try_get("metadata")?,
            });
        }

        // Active relationships — join subject/object entities to resolve names.
        let rel_rows = sqlx::query(
            "SELECT s.name AS subject_name, s.entity_type AS subject_type, \
                    o.name AS object_name, o.entity_type AS object_type, \
                    r.predicate, r.relationship_type, r.weight, r.confidence \
             FROM entity_relationships r \
             JOIN entities s ON r.subject_id = s.id \
             JOIN entities o ON r.object_id = o.id \
             WHERE r.invalid_at IS NULL \
             ORDER BY r.created_at",
        )
        .fetch_all(pool)
        .await?;

        let mut relationships = Vec::with_capacity(rel_rows.len());
        for row in rel_rows {
            relationships.push(ExportableRelationship {
                subject_name: row.try_get("subject_name")?,
                subject_type: row.try_get("subject_type")?,
                object_name: row.try_get("object_name")?,
                object_type: row.try_get("object_type")?,
                predicate: row.try_get("predicate")?,
                relationship_type: row.try_get("relationship_type")?,
                weight: row.try_get("weight")?,
                confidence: row.try_get("confidence")?,
            });
        }

        // Active facts — join to entities to resolve name.
        let fact_rows = sqlx::query(
            "SELECT e.name AS entity_name, e.entity_type, \
                    f.attribute, f.value, f.confidence \
             FROM entity_facts f \
             JOIN entities e ON f.entity_id = e.id \
             WHERE f.invalid_at IS NULL \
             ORDER BY f.created_at",
        )
        .fetch_all(pool)
        .await?;

        let mut facts = Vec::with_capacity(fact_rows.len());
        for row in fact_rows {
            facts.push(ExportableFact {
                entity_name: row.try_get("entity_name")?,
                entity_type: row.try_get("entity_type")?,
                attribute: row.try_get("attribute")?,
                value: row.try_get("value")?,
                confidence: row.try_get("confidence")?,
            });
        }

        Ok(ExportableGraph {
            entities,
            relationships,
            facts,
        })
    }

    /// Execute query without embedding join.
    async fn execute_query_no_embeddings(
        &self,
        pool: &sqlx::PgPool,
        sql: &str,
        project: Option<String>,
        since: Option<DateTime<Utc>>,
        tag_params: Vec<serde_json::Value>,
    ) -> Result<Vec<ExportableMemory>> {
        use sqlx::Row;

        // Bind positional params: project ($1), since ($2), then tag jsonb values.
        let mut q = sqlx::query(sql);
        if let Some(p) = project {
            q = q.bind(p);
        }
        if let Some(s) = since {
            q = q.bind(s);
        }
        for tag_val in tag_params {
            q = q.bind(tag_val);
        }
        let rows = q.fetch_all(pool).await?;

        let mut memories = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let content: String = row.try_get("content")?;
            let type_hint: String = row.try_get("type_hint")?;
            let source: String = row.try_get("source")?;
            let tags: Option<Value> = row.try_get("tags")?;
            let created_at: DateTime<Utc> = row.try_get("created_at")?;
            let actor: Option<String> = row.try_get("actor")?;
            let actor_type: String = row.try_get("actor_type")?;
            let audience: String = row.try_get("audience")?;
            let project: Option<String> = row.try_get("project")?;
            let event_time: Option<DateTime<Utc>> = row.try_get("event_time")?;
            let event_time_precision: Option<String> = row.try_get("event_time_precision")?;
            let stability: Option<f64> = row.try_get("stability")?;
            let difficulty: Option<f64> = row.try_get("difficulty")?;
            let reinforcement_count: Option<i64> = row.try_get("reinforcement_count")?;
            let last_reinforced_at: Option<DateTime<Utc>> = row.try_get("last_reinforced_at")?;

            memories.push(ExportableMemory {
                id,
                content,
                type_hint,
                source,
                tags,
                created_at,
                actor,
                actor_type,
                audience,
                project,
                event_time,
                event_time_precision,
                stability,
                difficulty,
                reinforcement_count,
                last_reinforced_at,
                embedding: None,
                embedding_model: None,
            });
        }

        Ok(memories)
    }

    /// Execute query with embedding join, parsing the pgvector embedding as Vec<f32>.
    async fn execute_query_with_embeddings(
        &self,
        pool: &sqlx::PgPool,
        sql: &str,
        project: Option<String>,
        since: Option<DateTime<Utc>>,
        tag_params: Vec<serde_json::Value>,
    ) -> Result<Vec<ExportableMemory>> {
        use sqlx::Row;

        let mut q = sqlx::query(sql);
        if let Some(p) = project {
            q = q.bind(p);
        }
        if let Some(s) = since {
            q = q.bind(s);
        }
        for tag_val in tag_params {
            q = q.bind(tag_val);
        }
        let rows = q.fetch_all(pool).await?;

        let mut memories = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            let content: String = row.try_get("content")?;
            let type_hint: String = row.try_get("type_hint")?;
            let source: String = row.try_get("source")?;
            let tags: Option<Value> = row.try_get("tags")?;
            let created_at: DateTime<Utc> = row.try_get("created_at")?;
            let actor: Option<String> = row.try_get("actor")?;
            let actor_type: String = row.try_get("actor_type")?;
            let audience: String = row.try_get("audience")?;
            let project: Option<String> = row.try_get("project")?;
            let event_time: Option<DateTime<Utc>> = row.try_get("event_time")?;
            let event_time_precision: Option<String> = row.try_get("event_time_precision")?;
            let stability: Option<f64> = row.try_get("stability")?;
            let difficulty: Option<f64> = row.try_get("difficulty")?;
            let reinforcement_count: Option<i64> = row.try_get("reinforcement_count")?;
            let last_reinforced_at: Option<DateTime<Utc>> = row.try_get("last_reinforced_at")?;

            // Parse embedding from text representation "[0.1,0.2,...]".
            let embedding_text: Option<String> = row.try_get("embedding_text")?;
            let embedding_model: Option<String> = row.try_get("embedding_model")?;

            let embedding = if let Some(ref text) = embedding_text {
                parse_pgvector_text(text)
            } else {
                None
            };

            memories.push(ExportableMemory {
                id,
                content,
                type_hint,
                source,
                tags,
                created_at,
                actor,
                actor_type,
                audience,
                project,
                event_time,
                event_time_precision,
                stability,
                difficulty,
                reinforcement_count,
                last_reinforced_at,
                embedding,
                embedding_model,
            });
        }

        Ok(memories)
    }
}

/// Parse a pgvector text representation "[0.1,0.2,...]" into Vec<f32>.
fn parse_pgvector_text(s: &str) -> Option<Vec<f32>> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return None;
    }
    let floats: Vec<f32> = trimmed
        .split(',')
        .filter_map(|part| part.trim().parse::<f32>().ok())
        .collect();
    if floats.is_empty() {
        None
    } else {
        Some(floats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_format_from_str() {
        assert_eq!(
            ExportFormat::from_str("jsonl").unwrap(),
            ExportFormat::Jsonl
        );
        assert_eq!(
            ExportFormat::from_str("JSONL").unwrap(),
            ExportFormat::Jsonl
        );
        assert_eq!(ExportFormat::from_str("csv").unwrap(), ExportFormat::Csv);
        assert_eq!(
            ExportFormat::from_str("markdown").unwrap(),
            ExportFormat::Markdown
        );
        assert_eq!(
            ExportFormat::from_str("md").unwrap(),
            ExportFormat::Markdown
        );
        assert!(ExportFormat::from_str("unknown").is_err());
    }

    #[test]
    fn test_parse_pgvector_text() {
        let result = parse_pgvector_text("[0.1,0.2,0.3]").unwrap();
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.1_f32).abs() < 1e-6);
        assert!((result[1] - 0.2_f32).abs() < 1e-6);
        assert!((result[2] - 0.3_f32).abs() < 1e-6);
    }

    #[test]
    fn test_parse_pgvector_text_empty() {
        assert!(parse_pgvector_text("[]").is_none());
        assert!(parse_pgvector_text("").is_none());
    }

    #[test]
    fn test_export_opts_default() {
        let opts = ExportOpts::default();
        assert_eq!(opts.format, ExportFormat::Jsonl);
        assert!(!opts.include_embeddings);
        assert!(!opts.include_state);
        assert!(!opts.include_graph);
        assert!(opts.project.is_none());
        assert!(opts.output.is_none());
    }
}
