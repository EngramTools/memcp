#![allow(clippy::unwrap_used)]
//! PostgreSQL-backed implementation of MemoryStore
//!
//! Uses sqlx with PgPool for connection pooling and production-grade persistence.
//! Supports optional migration execution on startup.
//!
//! Split into focused submodules:
//! - `queries` — MemoryStore trait impl (core CRUD)
//! - `embedding` — Embedding operations and vector search
//! - `salience` — Salience scoring, weight management, recall
//! - `extraction` — Content extraction, enrichment, GC, curation, sessions

mod embedding;
mod extraction;
mod graph;
mod queries;
mod salience;

use chrono::{DateTime, Utc};
use sqlx::{
    postgres::{PgPool, PgPoolOptions, PgRow},
    Connection as _, Row,
};
use std::time::Duration;

use crate::config::{IdempotencyConfig, SearchConfig};
use crate::errors::MemcpError;
use crate::store::Memory;

/// FSRS state row fetched from memory_salience table.
///
/// Missing rows are represented as defaults (stability=1.0, difficulty=5.0, count=0).
#[derive(Debug, Clone)]
pub struct SalienceRow {
    pub stability: f64,
    pub difficulty: f64,
    pub reinforcement_count: i32,
    pub last_reinforced_at: Option<DateTime<Utc>>,
}

impl Default for SalienceRow {
    fn default() -> Self {
        SalienceRow {
            stability: 1.0,
            difficulty: 5.0,
            reinforcement_count: 0,
            last_reinforced_at: None,
        }
    }
}

/// Context about memories related to a given memory by shared tags.
///
/// Returned by `get_related_context` — used by `cmd_recall` to build
/// per-memory hints pointing agents toward related content.
#[derive(Debug, Clone)]
pub struct RelatedContext {
    /// Number of other live memories sharing at least one non-trivial tag.
    pub related_count: i64,
    /// The shared tags (filtered — trivial tags excluded).
    pub shared_tags: Vec<String>,
}

/// Candidate memory returned by query-based recall (vector search).
///
/// Includes trust_level so the recall engine can apply trust weighting
/// without a second round-trip.
#[derive(Debug, Clone)]
pub struct RecallCandidate {
    pub memory_id: String,
    pub content: String,
    pub relevance: f32,
    pub tags: Option<serde_json::Value>,
    pub trust_level: f32,
    pub knowledge_tier: String,
    pub source_ids: Option<serde_json::Value>,
}

/// Candidate memory returned by query-less recall (no vector search).
///
/// Co-fetches salience data in a single query so the recall engine can
/// run `SalienceScorer::rank()` without a second round-trip.
#[derive(Debug, Clone)]
pub struct QuerylessCandidate {
    pub memory_id: String,
    pub content: String,
    pub updated_at: chrono::DateTime<Utc>,
    pub access_count: i64,
    pub stability: f64,
    pub last_reinforced_at: Option<chrono::DateTime<Utc>>,
    pub tags: Option<serde_json::Value>,
    pub trust_level: f32,
    pub knowledge_tier: String,
    pub source_ids: Option<serde_json::Value>,
}

/// A curation run record from the curation_runs table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CurationRunRow {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: String,
    pub mode: String,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: DateTime<Utc>,
    pub merged_count: i32,
    pub flagged_stale_count: i32,
    pub strengthened_count: i32,
    pub skipped_count: i32,
    pub error_message: Option<String>,
}

/// A curation action record from the curation_actions table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CurationActionRow {
    pub id: String,
    pub run_id: String,
    pub action_type: String,
    pub target_memory_ids: Vec<String>,
    pub merged_memory_id: Option<String>,
    pub original_salience: Option<f64>,
    pub details: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// PostgreSQL-backed memory store using sqlx connection pool.
pub struct PostgresMemoryStore {
    pub(crate) pool: PgPool,
    /// Whether the ParadeDB pg_search extension is installed on this PostgreSQL instance.
    /// Detected once at construction time via pg_extension catalog query.
    pub(crate) paradedb_available: bool,
    /// Whether to use ParadeDB for BM25 search (paradedb_available AND config says "paradedb").
    pub(crate) use_paradedb: bool,
    /// Configured embedding dimension, set by daemon after provider initialization.
    /// None when created by CLI (no embedding provider loaded).
    /// Used for explicit casts in vector queries when needed.
    pub(crate) embedding_dimension: Option<usize>,
    /// Idempotency configuration: dedup window, key TTL, max key length.
    pub(crate) idempotency_config: IdempotencyConfig,
    /// Optional schema name for isolation (e.g. "benchmark").
    /// When set, all pool connections use SET search_path TO {schema}, public.
    /// When None, uses the default public schema.
    pub(crate) schema: Option<String>,
    /// Retention configuration: type-specific initial FSRS stability.
    /// When set, store() applies type-appropriate initial stability after INSERT.
    pub(crate) retention_config: Option<crate::config::RetentionConfig>,
}

impl PostgresMemoryStore {
    /// Create a new PostgresMemoryStore, connecting to the PostgreSQL database at database_url.
    ///
    /// Configures a production-ready connection pool with sensible defaults.
    /// If run_migrations is true, automatically runs pending migrations on startup.
    /// Detects ParadeDB pg_search extension at startup and caches result.
    pub async fn new(database_url: &str, run_migrations: bool) -> Result<Self, MemcpError> {
        Self::new_with_schema(database_url, run_migrations, &SearchConfig::default(), None).await
    }

    /// Create a new PostgresMemoryStore with an explicit SearchConfig.
    ///
    /// Allows operators to set bm25_backend via config or env var.
    pub async fn new_with_search_config(
        database_url: &str,
        run_migrations: bool,
        search_config: &SearchConfig,
    ) -> Result<Self, MemcpError> {
        Self::new_with_schema(database_url, run_migrations, search_config, None).await
    }

    /// Create a new PostgresMemoryStore with a configurable connection pool size.
    ///
    /// Use this constructor in daemon mode to wire `max_db_connections` from config.
    /// All other callers use `new()` which defaults to 10 connections.
    pub async fn new_with_pool_config(
        database_url: &str,
        run_migrations: bool,
        search_config: &SearchConfig,
        max_connections: u32,
    ) -> Result<Self, MemcpError> {
        Self::new_with_schema_internal(
            database_url,
            run_migrations,
            search_config,
            None,
            max_connections,
        )
        .await
    }

    /// Create a new PostgresMemoryStore with optional schema isolation.
    ///
    /// When `schema` is `Some(name)`, all pool connections use `SET search_path TO {name}, public`
    /// so that migrations and queries operate in the target schema rather than public.
    /// `public` is kept in the search_path so extensions (pgvector, pg_trgm) remain accessible.
    ///
    /// When `schema` is `None`, behaves identically to the existing constructors (public schema).
    ///
    /// The schema name must be alphanumeric + underscore only to prevent SQL injection.
    pub async fn new_with_schema(
        database_url: &str,
        run_migrations: bool,
        search_config: &SearchConfig,
        schema: Option<&str>,
    ) -> Result<Self, MemcpError> {
        Self::new_with_schema_internal(database_url, run_migrations, search_config, schema, 10)
            .await
    }

    /// Internal constructor with full parameter control.
    async fn new_with_schema_internal(
        database_url: &str,
        run_migrations: bool,
        search_config: &SearchConfig,
        schema: Option<&str>,
        max_connections: u32,
    ) -> Result<Self, MemcpError> {
        // Validate schema name if provided
        if let Some(name) = schema {
            if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(MemcpError::Storage(format!(
                    "Invalid schema name '{}': must be alphanumeric/underscore only",
                    name
                )));
            }
        }

        // If schema requested, create it via a one-off connection before building the pool
        if let Some(name) = schema {
            let mut conn = sqlx::postgres::PgConnection::connect(database_url)
                .await
                .map_err(|e| {
                    MemcpError::Storage(format!("Failed to connect for schema creation: {}", e))
                })?;
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{}\"", name))
                .execute(&mut conn)
                .await
                .map_err(|e| {
                    MemcpError::Storage(format!("Failed to create schema '{}': {}", name, e))
                })?;
            conn.close().await.ok();
        }

        // Build pool with optional search_path hook for schema isolation
        let schema_owned: Option<String> = schema.map(|s| s.to_string());
        let pool = {
            let mut opts = PgPoolOptions::new()
                .max_connections(max_connections) // configurable: daemon uses resource_caps.max_db_connections
                .min_connections(1) // keep at least one warm connection
                .idle_timeout(Duration::from_secs(300)) // 5 min idle cleanup
                .max_lifetime(Duration::from_secs(1800)); // 30 min max connection age

            if let Some(ref schema_name) = schema_owned {
                let s = schema_name.clone();
                opts = opts.after_connect(move |conn, _meta| {
                    let schema = s.clone();
                    Box::pin(async move {
                        sqlx::query(&format!("SET search_path TO \"{}\", public", schema))
                            .execute(&mut *conn)
                            .await?;
                        Ok(())
                    })
                });
            }

            opts.connect(database_url)
                .await
                .map_err(|e| MemcpError::Storage(format!("Failed to connect to database: {}", e)))?
        };

        if run_migrations {
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .map_err(|e| MemcpError::Storage(format!("Migration failed: {}", e)))?;
        }

        // Detect ParadeDB at startup — cached as bool for the lifetime of the store
        let paradedb_available = Self::detect_paradedb(&pool).await;

        // Determine effective BM25 backend:
        // - "paradedb" config + available → use ParadeDB
        // - "paradedb" config + NOT available → warn, fall back to native
        // - "native" config (default) → always use native
        let use_paradedb = if search_config.bm25_backend == "paradedb" {
            if paradedb_available {
                tracing::info!("ParadeDB pg_search extension detected — using ParadeDB for BM25");
                true
            } else {
                tracing::warn!(
                    "bm25_backend=paradedb configured but pg_search extension not found — falling back to native PostgreSQL tsvector"
                );
                false
            }
        } else {
            if paradedb_available {
                tracing::info!("ParadeDB pg_search extension detected — using native PostgreSQL tsvector for BM25 (set bm25_backend=paradedb to opt in)");
            } else {
                tracing::info!("Using native PostgreSQL tsvector for BM25");
            }
            false
        };

        Ok(PostgresMemoryStore {
            pool,
            paradedb_available,
            use_paradedb,
            embedding_dimension: None,
            idempotency_config: IdempotencyConfig::default(),
            schema: schema_owned,
            retention_config: None,
        })
    }

    /// Drop the schema used by this store (CASCADE).
    ///
    /// Only valid when the store was created with a schema via `new_with_schema()`.
    /// Returns an error if no schema was set.
    /// Used by the benchmark binary to clean up the ephemeral benchmark schema after a run.
    pub async fn drop_schema(&self) -> Result<(), MemcpError> {
        if let Some(ref schema_name) = self.schema {
            sqlx::query(&format!(
                "DROP SCHEMA IF EXISTS \"{}\" CASCADE",
                schema_name
            ))
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to drop schema '{}': {}", schema_name, e))
            })?;
            tracing::info!(schema = %schema_name, "Dropped benchmark schema");
            Ok(())
        } else {
            Err(MemcpError::Storage(
                "No schema set — cannot drop".to_string(),
            ))
        }
    }

    /// Create a PostgresMemoryStore from an existing connection pool.
    ///
    /// Used by `#[sqlx::test]` which manages database lifecycle (create, migrate, drop)
    /// and provides a pre-configured pool pointing to an ephemeral test database.
    pub async fn from_pool(pool: PgPool) -> Result<Self, MemcpError> {
        let search_config = SearchConfig::default();
        let paradedb_available = Self::detect_paradedb(&pool).await;
        let use_paradedb = if search_config.bm25_backend == "paradedb" {
            paradedb_available
        } else {
            false
        };
        Ok(Self {
            pool,
            paradedb_available,
            use_paradedb,
            embedding_dimension: None,
            idempotency_config: IdempotencyConfig::default(),
            schema: None,
            retention_config: None,
        })
    }

    /// Create a PostgresMemoryStore from an existing pool with an explicit IdempotencyConfig.
    ///
    /// Used in production paths where the full Config is available.
    pub async fn from_pool_with_idempotency(
        pool: PgPool,
        idempotency_config: IdempotencyConfig,
    ) -> Result<Self, MemcpError> {
        let search_config = SearchConfig::default();
        let paradedb_available = Self::detect_paradedb(&pool).await;
        let use_paradedb = if search_config.bm25_backend == "paradedb" {
            paradedb_available
        } else {
            false
        };
        Ok(Self {
            pool,
            paradedb_available,
            use_paradedb,
            embedding_dimension: None,
            idempotency_config,
            schema: None,
            retention_config: None,
        })
    }

    /// Update the idempotency configuration after construction.
    ///
    /// Typically called after loading the full Config, before the store handles requests.
    pub fn set_idempotency_config(&mut self, config: IdempotencyConfig) {
        self.idempotency_config = config;
    }

    /// Update the retention configuration after construction.
    ///
    /// When set, store() applies type-specific initial FSRS stability based on type_hint.
    /// Typically called from main.rs or daemon after loading the full Config.
    pub fn set_retention_config(&mut self, config: crate::config::RetentionConfig) {
        self.retention_config = Some(config);
    }

    /// Truncate all benchmark-relevant tables: memories, memory_embeddings, memory_salience, memory_consolidations.
    /// Uses TRUNCATE ... CASCADE for speed. Benchmark-only — not exposed via MCP.
    /// Retries up to 5 times on deadlock (embedding worker may hold locks).
    ///
    /// Safety: Requires a named schema (e.g. "benchmark"). Refuses to operate on the
    /// public schema to prevent accidental production data destruction.
    pub async fn truncate_all(&self) -> Result<(), MemcpError> {
        if self.schema.is_none() {
            return Err(MemcpError::Storage(
                "truncate_all() requires a named schema for safety. \
                 Use new_with_schema() to isolate destructive operations."
                    .to_string(),
            ));
        }
        for attempt in 0..5u32 {
            match sqlx::query("TRUNCATE memories, memory_embeddings, memory_salience, memory_consolidations CASCADE")
                .execute(&self.pool)
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) if e.to_string().contains("deadlock") && attempt < 4 => {
                    let delay = std::time::Duration::from_millis(200 * 2u64.pow(attempt));
                    tracing::warn!(attempt = attempt + 1, delay_ms = delay.as_millis(), "Truncate deadlock, retrying");
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(MemcpError::Storage(format!("Failed to truncate tables: {}", e))),
            }
        }
        unreachable!()
    }

    /// Detect whether the ParadeDB pg_search extension is installed on this PostgreSQL instance.
    ///
    /// Queries the pg_extension catalog once at startup. Returns true if pg_search is present.
    async fn detect_paradedb(pool: &PgPool) -> bool {
        sqlx::query("SELECT 1 FROM pg_extension WHERE extname = 'pg_search' LIMIT 1")
            .fetch_optional(pool)
            .await
            .is_ok_and(|r| r.is_some())
    }

    /// Set the configured embedding dimension.
    ///
    /// Called by daemon after the embedding provider is initialized.
    /// Stored for use in query casts when needed.
    pub fn set_embedding_dimension(&mut self, dimension: usize) {
        self.embedding_dimension = Some(dimension);
    }

    /// Ensure the HNSW index exists with the correct dimension-aware cast.
    ///
    /// Called at daemon startup after the embedding provider is initialized.
    /// The index uses `(embedding::vector(N))` to cast the untyped column to the
    /// configured dimension so pgvector can apply cosine ops.
    ///
    /// If the index already exists (e.g., daemon restarted), this is a no-op.
    pub async fn ensure_hnsw_index(&self, dimension: usize) -> Result<(), MemcpError> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_indexes WHERE indexname = 'idx_memory_embeddings_hnsw')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to check HNSW index: {}", e)))?;

        if !exists {
            let sql = format!(
                "CREATE INDEX idx_memory_embeddings_hnsw ON memory_embeddings \
                 USING hnsw ((embedding::vector({})) vector_cosine_ops) \
                 WITH (m = 16, ef_construction = 64)",
                dimension
            );
            sqlx::query(&sql)
                .execute(&self.pool)
                .await
                .map_err(|e| MemcpError::Storage(format!("Failed to create HNSW index: {}", e)))?;
            tracing::info!(dimension, "Created HNSW index for vector dimension");
        } else {
            tracing::debug!(dimension, "HNSW index already exists, skipping creation");
        }

        Ok(())
    }

    /// Drop the HNSW index.
    ///
    /// Useful for tests and model migration scenarios where the index needs
    /// to be rebuilt for a different dimension.
    pub async fn drop_hnsw_index(&self) -> Result<(), MemcpError> {
        sqlx::query("DROP INDEX IF EXISTS idx_memory_embeddings_hnsw")
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to drop HNSW index: {}", e)))?;
        Ok(())
    }

    /// Ensure the HNSW index exists for a specific embedding tier.
    ///
    /// Creates a partial index filtered by `tier` so that each tier's embeddings
    /// use a dimension-appropriate HNSW index. The partial index allows pgvector
    /// to use the correct vector cast for each tier's dimension.
    pub async fn ensure_hnsw_index_for_tier(
        &self,
        tier: &str,
        dimension: usize,
    ) -> Result<(), MemcpError> {
        let index_name = format!("idx_memory_embeddings_hnsw_{}", tier);
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_indexes WHERE indexname = $1)")
                .bind(&index_name)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    MemcpError::Storage(format!(
                        "Failed to check HNSW index for tier {}: {}",
                        tier, e
                    ))
                })?;

        if !exists {
            let sql = format!(
                "CREATE INDEX {} ON memory_embeddings \
                 USING hnsw ((embedding::vector({})) vector_cosine_ops) \
                 WHERE tier = '{}' \
                 WITH (m = 16, ef_construction = 64)",
                index_name, dimension, tier
            );
            sqlx::query(&sql).execute(&self.pool).await.map_err(|e| {
                MemcpError::Storage(format!(
                    "Failed to create HNSW index for tier {}: {}",
                    tier, e
                ))
            })?;
            tracing::info!(tier, dimension, "Created HNSW index for embedding tier");
        } else {
            tracing::debug!(
                tier,
                dimension,
                "HNSW index for tier already exists, skipping"
            );
        }

        Ok(())
    }

    /// Count the number of current embeddings in a specific tier.
    ///
    /// Used for lazy quality query embedding: skip API call when no memories
    /// use the quality tier yet.
    pub async fn count_tier_embeddings(&self, tier: &str) -> Result<i64, MemcpError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memory_embeddings WHERE tier = $1 AND is_current = true",
        )
        .bind(tier)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to count tier embeddings: {}", e)))?;

        Ok(count)
    }

    /// Find memories eligible for promotion from one tier to another.
    ///
    /// Returns memory IDs that currently have embeddings in `current_tier` and
    /// meet the promotion thresholds (stability and reinforcement count from memory_salience).
    pub async fn get_promotion_candidates(
        &self,
        min_stability: f64,
        min_reinforcements: i32,
        current_tier: &str,
        limit: i64,
    ) -> Result<Vec<String>, MemcpError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT me.memory_id \
             FROM memory_embeddings me \
             JOIN memory_salience ms ON ms.memory_id = me.memory_id \
             JOIN memories m ON m.id = me.memory_id \
             WHERE me.tier = $1 \
               AND me.is_current = true \
               AND m.deleted_at IS NULL \
               AND (ms.stability >= $2 OR ms.reinforcement_count >= $3) \
             ORDER BY ms.stability DESC \
             LIMIT $4",
        )
        .bind(current_tier)
        .bind(min_stability)
        .bind(min_reinforcements)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get promotion candidates: {}", e)))?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Deactivate the current embedding for a memory in a specific tier.
    ///
    /// Sets `is_current = false` so that a new embedding in the target tier
    /// can take over. Used during promotion from fast to quality tier.
    pub async fn deactivate_tier_embedding(
        &self,
        memory_id: &str,
        tier: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE memory_embeddings SET is_current = false, updated_at = NOW() \
             WHERE memory_id = $1 AND tier = $2 AND is_current = true",
        )
        .bind(memory_id)
        .bind(tier)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to deactivate tier embedding: {}", e)))?;
        Ok(())
    }
}

/// Map a sqlx PgRow to a Memory struct.
///
/// PostgreSQL native types map directly:
/// - TIMESTAMPTZ -> DateTime<Utc> (no string parsing)
/// - JSONB -> Option<serde_json::Value> (no string parsing)
///
/// New extraction and consolidation columns are read with defaults when absent
/// (e.g., rows from JOIN queries that don't select these columns).
pub(crate) fn row_to_memory(row: &PgRow) -> Result<Memory, MemcpError> {
    Ok(Memory {
        id: row
            .try_get("id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        content: row
            .try_get("content")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        type_hint: row
            .try_get("type_hint")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        source: row
            .try_get("source")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        tags: row
            .try_get("tags")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        last_accessed_at: row
            .try_get("last_accessed_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        access_count: row
            .try_get("access_count")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        embedding_status: row
            .try_get("embedding_status")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        extracted_entities: row.try_get("extracted_entities").unwrap_or(None),
        extracted_facts: row.try_get("extracted_facts").unwrap_or(None),
        extraction_status: row
            .try_get("extraction_status")
            .unwrap_or_else(|_| "pending".to_string()),
        is_consolidated_original: row.try_get("is_consolidated_original").unwrap_or(false),
        consolidated_into: row.try_get("consolidated_into").unwrap_or(None),
        actor: row.try_get("actor").unwrap_or(None),
        actor_type: row
            .try_get("actor_type")
            .unwrap_or_else(|_| "agent".to_string()),
        audience: row
            .try_get("audience")
            .unwrap_or_else(|_| "global".to_string()),
        event_time: row.try_get("event_time").unwrap_or(None),
        event_time_precision: row.try_get("event_time_precision").unwrap_or(None),
        project: row.try_get("project").unwrap_or(None),
        trust_level: row.try_get("trust_level").unwrap_or(0.5),
        session_id: row.try_get("session_id").unwrap_or(None),
        agent_role: row.try_get("agent_role").unwrap_or(None),
        write_path: row.try_get("write_path").unwrap_or(None),
        metadata: row
            .try_get("metadata")
            .unwrap_or_else(|_| serde_json::json!({})),
        abstract_text: row.try_get("abstract_text").unwrap_or(None),
        overview_text: row.try_get("overview_text").unwrap_or(None),
        abstraction_status: row
            .try_get("abstraction_status")
            .unwrap_or_else(|_| "pending".to_string()),
        knowledge_tier: row
            .try_get("knowledge_tier")
            .unwrap_or_else(|_| "explicit".to_string()),
        source_ids: row.try_get("source_ids").unwrap_or(None),
        reply_to_id: row.try_get("reply_to_id").unwrap_or(None),
    })
}
