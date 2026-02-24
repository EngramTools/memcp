/// PostgreSQL-backed implementation of MemoryStore
///
/// Uses sqlx with PgPool for connection pooling and production-grade persistence.
/// Supports optional migration execution on startup.

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde_json;
use sqlx::{
    postgres::{PgPool, PgPoolOptions, PgRow},
    Row,
};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use crate::config::SearchConfig;
use crate::errors::MemcpError;
use crate::store::{
    decode_search_keyset_cursor, encode_search_keyset_cursor,
    CreateMemory, ListFilter, ListResult, Memory, MemoryStore,
    SearchFilter, SearchHit, SearchResult, UpdateMemory,
};

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

/// PostgreSQL-backed memory store using sqlx connection pool.
pub struct PostgresMemoryStore {
    pool: PgPool,
    /// Whether the ParadeDB pg_search extension is installed on this PostgreSQL instance.
    /// Detected once at construction time via pg_extension catalog query.
    paradedb_available: bool,
    /// Whether to use ParadeDB for BM25 search (paradedb_available AND config says "paradedb").
    use_paradedb: bool,
    /// Configured embedding dimension, set by daemon after provider initialization.
    /// None when created by CLI (no embedding provider loaded).
    /// Used for explicit casts in vector queries when needed.
    embedding_dimension: Option<usize>,
}

impl PostgresMemoryStore {
    /// Create a new PostgresMemoryStore, connecting to the PostgreSQL database at database_url.
    ///
    /// Configures a production-ready connection pool with sensible defaults.
    /// If run_migrations is true, automatically runs pending migrations on startup.
    /// Detects ParadeDB pg_search extension at startup and caches result.
    pub async fn new(database_url: &str, run_migrations: bool) -> Result<Self, MemcpError> {
        Self::new_with_search_config(database_url, run_migrations, &SearchConfig::default()).await
    }

    /// Create a new PostgresMemoryStore with an explicit SearchConfig.
    ///
    /// Allows operators to set bm25_backend via config or env var.
    pub async fn new_with_search_config(
        database_url: &str,
        run_migrations: bool,
        search_config: &SearchConfig,
    ) -> Result<Self, MemcpError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)         // good default for single-server MCP stdio
            .min_connections(1)          // keep at least one warm connection
            .idle_timeout(Duration::from_secs(300))    // 5 min idle cleanup
            .max_lifetime(Duration::from_secs(1800))   // 30 min max connection age
            .connect(database_url)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to connect to database: {}", e)))?;

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

        Ok(PostgresMemoryStore { pool, paradedb_available, use_paradedb, embedding_dimension: None })
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
        })
    }

    /// Truncate all benchmark-relevant tables: memories, memory_embeddings, memory_salience, memory_consolidations.
    /// Uses TRUNCATE ... CASCADE for speed. Benchmark-only — not exposed via MCP.
    pub async fn truncate_all(&self) -> Result<(), MemcpError> {
        sqlx::query("TRUNCATE memories, memory_embeddings, memory_salience, memory_consolidations CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to truncate tables: {}", e)))?;
        Ok(())
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
}

/// Encode a pagination cursor from created_at and id.
fn encode_cursor(created_at: &DateTime<Utc>, id: &str) -> String {
    let raw = format!("{}|{}", created_at.to_rfc3339(), id);
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

/// Decode a pagination cursor back into (created_at, id).
fn decode_cursor(cursor: &str) -> Result<(DateTime<Utc>, String), MemcpError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|e| MemcpError::Validation {
            message: format!("Invalid cursor encoding: {}", e),
            field: Some("cursor".to_string()),
        })?;
    let raw = String::from_utf8(bytes).map_err(|e| MemcpError::Validation {
        message: format!("Invalid cursor content: {}", e),
        field: Some("cursor".to_string()),
    })?;
    let mut parts = raw.splitn(2, '|');
    let ts_str = parts.next().ok_or_else(|| MemcpError::Validation {
        message: "Cursor missing timestamp".to_string(),
        field: Some("cursor".to_string()),
    })?;
    let id_str = parts.next().ok_or_else(|| MemcpError::Validation {
        message: "Cursor missing id".to_string(),
        field: Some("cursor".to_string()),
    })?;
    let created_at = ts_str
        .parse::<DateTime<Utc>>()
        .map_err(|e| MemcpError::Validation {
            message: format!("Cursor timestamp parse error: {}", e),
            field: Some("cursor".to_string()),
        })?;
    Ok((created_at, id_str.to_string()))
}

/// Map a sqlx PgRow to a Memory struct.
///
/// PostgreSQL native types map directly:
/// - TIMESTAMPTZ -> DateTime<Utc> (no string parsing)
/// - JSONB -> Option<serde_json::Value> (no string parsing)
///
/// New extraction and consolidation columns are read with defaults when absent
/// (e.g., rows from JOIN queries that don't select these columns).
fn row_to_memory(row: &PgRow) -> Result<Memory, MemcpError> {
    Ok(Memory {
        id: row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))?,
        content: row.try_get("content").map_err(|e| MemcpError::Storage(e.to_string()))?,
        type_hint: row.try_get("type_hint").map_err(|e| MemcpError::Storage(e.to_string()))?,
        source: row.try_get("source").map_err(|e| MemcpError::Storage(e.to_string()))?,
        tags: row.try_get("tags").map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row.try_get("created_at").map_err(|e| MemcpError::Storage(e.to_string()))?,
        updated_at: row.try_get("updated_at").map_err(|e| MemcpError::Storage(e.to_string()))?,
        last_accessed_at: row.try_get("last_accessed_at").map_err(|e| MemcpError::Storage(e.to_string()))?,
        access_count: row.try_get("access_count").map_err(|e| MemcpError::Storage(e.to_string()))?,
        embedding_status: row.try_get("embedding_status").map_err(|e| MemcpError::Storage(e.to_string()))?,
        extracted_entities: row.try_get("extracted_entities").unwrap_or(None),
        extracted_facts: row.try_get("extracted_facts").unwrap_or(None),
        extraction_status: row.try_get("extraction_status").unwrap_or_else(|_| "pending".to_string()),
        is_consolidated_original: row.try_get("is_consolidated_original").unwrap_or(false),
        consolidated_into: row.try_get("consolidated_into").unwrap_or(None),
        actor: row.try_get("actor").unwrap_or(None),
        actor_type: row.try_get("actor_type").unwrap_or_else(|_| "agent".to_string()),
        audience: row.try_get("audience").unwrap_or_else(|_| "global".to_string()),
    })
}

#[async_trait]
impl MemoryStore for PostgresMemoryStore {
    async fn store(&self, input: CreateMemory) -> Result<Memory, MemcpError> {
        let id = Uuid::new_v4().to_string();
        let now = input.created_at.unwrap_or_else(Utc::now);

        // Convert tags Vec<String> to serde_json::Value for JSONB binding
        let tags_json: Option<serde_json::Value> = input
            .tags
            .as_ref()
            .map(|t| serde_json::json!(t));

        sqlx::query(
            "INSERT INTO memories (id, content, type_hint, source, tags, created_at, updated_at, access_count, embedding_status, actor, actor_type, audience) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'pending', $8, $9, $10)",
        )
        .bind(&id)
        .bind(&input.content)
        .bind(&input.type_hint)
        .bind(&input.source)
        .bind(&tags_json)     // JSONB — bind serde_json::Value directly
        .bind(&now)           // TIMESTAMPTZ — bind DateTime<Utc> directly
        .bind(&now)
        .bind(&input.actor)
        .bind(&input.actor_type)
        .bind(&input.audience)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to insert memory: {}", e)))?;

        Ok(Memory {
            id,
            content: input.content,
            type_hint: input.type_hint,
            source: input.source,
            tags: tags_json,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
            embedding_status: "pending".to_string(),
            extracted_entities: None,
            extracted_facts: None,
            extraction_status: "pending".to_string(),
            is_consolidated_original: false,
            consolidated_into: None,
            actor: input.actor,
            actor_type: input.actor_type,
            audience: input.audience,
        })
    }

    async fn get(&self, id: &str) -> Result<Memory, MemcpError> {
        let row = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience \
             FROM memories WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?
        .ok_or_else(|| MemcpError::NotFound { id: id.to_string() })?;

        let memory = row_to_memory(&row)?;

        // Fire-and-forget touch to update access stats
        let _ = self.touch(id).await;

        Ok(memory)
    }

    async fn update(&self, id: &str, input: UpdateMemory) -> Result<Memory, MemcpError> {
        // Verify the memory exists first
        let row = sqlx::query("SELECT id FROM memories WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        if row.is_none() {
            return Err(MemcpError::NotFound { id: id.to_string() });
        }

        let now = Utc::now();

        // Build dynamic SET clause with numbered PostgreSQL parameters
        let mut param_idx: u32 = 1;
        let mut sets: Vec<String> = Vec::new();

        // updated_at is always set
        sets.push(format!("updated_at = ${}", param_idx));
        param_idx += 1;

        if input.content.is_some() {
            sets.push(format!("content = ${}", param_idx));
            param_idx += 1;
        }
        if input.type_hint.is_some() {
            sets.push(format!("type_hint = ${}", param_idx));
            param_idx += 1;
        }
        if input.source.is_some() {
            sets.push(format!("source = ${}", param_idx));
            param_idx += 1;
        }
        if input.tags.is_some() {
            sets.push(format!("tags = ${}", param_idx));
            param_idx += 1;
        }

        let sql = format!(
            "UPDATE memories SET {} WHERE id = ${}",
            sets.join(", "),
            param_idx
        );

        let mut q = sqlx::query(&sql).bind(&now); // $1 = updated_at
        if let Some(ref content) = input.content {
            q = q.bind(content);
        }
        if let Some(ref type_hint) = input.type_hint {
            q = q.bind(type_hint);
        }
        if let Some(ref source) = input.source {
            q = q.bind(source);
        }
        if let Some(ref tags) = input.tags {
            // Convert Vec<String> to serde_json::Value for JSONB
            let tags_json = serde_json::json!(tags);
            q = q.bind(tags_json);
        }
        q = q.bind(id); // final $N = id

        q.execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to update memory: {}", e)))?;

        // Re-fetch and return the updated record
        let updated_row = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience \
             FROM memories WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        row_to_memory(&updated_row)
    }

    async fn delete(&self, id: &str) -> Result<(), MemcpError> {
        let result = sqlx::query("DELETE FROM memories WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(MemcpError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    async fn list(&self, filter: ListFilter) -> Result<ListResult, MemcpError> {
        let limit = filter.limit.min(100).max(1);

        // Build WHERE clause with numbered PostgreSQL parameters
        let mut conditions: Vec<String> = vec!["deleted_at IS NULL".to_string()];
        let mut param_idx: u32 = 1;
        let mut cursor_created_at: Option<DateTime<Utc>> = None;
        let mut cursor_id: Option<String> = None;

        if filter.type_hint.is_some() {
            conditions.push(format!("type_hint = ${}", param_idx));
            param_idx += 1;
        }
        if filter.source.is_some() {
            // Prefix match: --source openclaw matches openclaw/vita, openclaw/main, etc.
            conditions.push(format!("source LIKE ${} || '%'", param_idx));
            param_idx += 1;
        }
        if filter.created_after.is_some() {
            conditions.push(format!("created_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_before.is_some() {
            conditions.push(format!("created_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_after.is_some() {
            conditions.push(format!("updated_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_before.is_some() {
            conditions.push(format!("updated_at < ${}", param_idx));
            param_idx += 1;
        }
        if let Some(ref cursor) = filter.cursor {
            let (ca, cid) = decode_cursor(cursor)?;
            cursor_created_at = Some(ca);
            cursor_id = Some(cid);
            // Cursor comparison uses 3 params: created_at < $N OR (created_at = $N+1 AND id > $N+2)
            conditions.push(format!(
                "(created_at < ${} OR (created_at = ${} AND id > ${}))",
                param_idx, param_idx + 1, param_idx + 2
            ));
            param_idx += 3;
        }
        if filter.actor.is_some() {
            conditions.push(format!("actor = ${}", param_idx));
            param_idx += 1;
        }
        if filter.audience.is_some() {
            conditions.push(format!("audience = ${}", param_idx));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience \
             FROM memories {} ORDER BY created_at DESC, id ASC LIMIT ${}",
            where_clause, param_idx
        );

        let mut q = sqlx::query(&sql);
        if let Some(ref th) = filter.type_hint {
            q = q.bind(th);
        }
        if let Some(ref src) = filter.source {
            q = q.bind(src);
        }
        if let Some(ref ca) = filter.created_after {
            q = q.bind(ca);
        }
        if let Some(ref cb) = filter.created_before {
            q = q.bind(cb);
        }
        if let Some(ref ua) = filter.updated_after {
            q = q.bind(ua);
        }
        if let Some(ref ub) = filter.updated_before {
            q = q.bind(ub);
        }
        if let Some(ref ca) = cursor_created_at {
            let cid = cursor_id.as_deref().unwrap_or("");
            // Bind 3 times for the cursor comparison: $N, $N+1 (same value), $N+2
            q = q.bind(ca).bind(ca).bind(cid.to_string());
        }
        if let Some(ref actor) = filter.actor {
            q = q.bind(actor);
        }
        if let Some(ref audience) = filter.audience {
            q = q.bind(audience);
        }
        // Fetch one extra to determine if there are more pages
        q = q.bind(limit + 1);

        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        let has_more = rows.len() as i64 > limit;
        let take = if has_more { limit as usize } else { rows.len() };
        let mut memories = Vec::with_capacity(take);

        for row in rows.iter().take(take) {
            memories.push(row_to_memory(row)?);
        }

        let next_cursor = if has_more {
            memories.last().map(|m| encode_cursor(&m.created_at, &m.id))
        } else {
            None
        };

        Ok(ListResult {
            memories,
            next_cursor,
        })
    }

    async fn count_matching(&self, filter: &ListFilter) -> Result<u64, MemcpError> {
        let mut conditions: Vec<String> = vec!["deleted_at IS NULL".to_string()];
        let mut param_idx: u32 = 1;

        if filter.type_hint.is_some() {
            conditions.push(format!("type_hint = ${}", param_idx));
            param_idx += 1;
        }
        if filter.source.is_some() {
            conditions.push(format!("source LIKE ${} || '%'", param_idx));
            param_idx += 1;
        }
        if filter.created_after.is_some() {
            conditions.push(format!("created_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_before.is_some() {
            conditions.push(format!("created_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_after.is_some() {
            conditions.push(format!("updated_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_before.is_some() {
            conditions.push(format!("updated_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.actor.is_some() {
            conditions.push(format!("actor = ${}", param_idx));
            param_idx += 1;
        }
        if filter.audience.is_some() {
            conditions.push(format!("audience = ${}", param_idx));
            let _ = param_idx + 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) as count FROM memories {}", where_clause);

        let mut q = sqlx::query(&sql);
        if let Some(ref th) = filter.type_hint {
            q = q.bind(th);
        }
        if let Some(ref src) = filter.source {
            q = q.bind(src);
        }
        if let Some(ref ca) = filter.created_after {
            q = q.bind(ca);
        }
        if let Some(ref cb) = filter.created_before {
            q = q.bind(cb);
        }
        if let Some(ref ua) = filter.updated_after {
            q = q.bind(ua);
        }
        if let Some(ref ub) = filter.updated_before {
            q = q.bind(ub);
        }
        if let Some(ref actor) = filter.actor {
            q = q.bind(actor);
        }
        if let Some(ref audience) = filter.audience {
            q = q.bind(audience);
        }

        let row = q
            .fetch_one(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        let count: i64 = row.try_get("count").map_err(|e| MemcpError::Storage(e.to_string()))?;
        Ok(count as u64)
    }

    async fn delete_matching(&self, filter: &ListFilter) -> Result<u64, MemcpError> {
        let mut conditions: Vec<String> = vec!["deleted_at IS NULL".to_string()];
        let mut param_idx: u32 = 1;

        if filter.type_hint.is_some() {
            conditions.push(format!("type_hint = ${}", param_idx));
            param_idx += 1;
        }
        if filter.source.is_some() {
            conditions.push(format!("source LIKE ${} || '%'", param_idx));
            param_idx += 1;
        }
        if filter.created_after.is_some() {
            conditions.push(format!("created_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_before.is_some() {
            conditions.push(format!("created_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_after.is_some() {
            conditions.push(format!("updated_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.updated_before.is_some() {
            conditions.push(format!("updated_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.actor.is_some() {
            conditions.push(format!("actor = ${}", param_idx));
            param_idx += 1;
        }
        if filter.audience.is_some() {
            conditions.push(format!("audience = ${}", param_idx));
            let _ = param_idx + 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("DELETE FROM memories {}", where_clause);

        let mut q = sqlx::query(&sql);
        if let Some(ref th) = filter.type_hint {
            q = q.bind(th);
        }
        if let Some(ref src) = filter.source {
            q = q.bind(src);
        }
        if let Some(ref ca) = filter.created_after {
            q = q.bind(ca);
        }
        if let Some(ref cb) = filter.created_before {
            q = q.bind(cb);
        }
        if let Some(ref ua) = filter.updated_after {
            q = q.bind(ua);
        }
        if let Some(ref ub) = filter.updated_before {
            q = q.bind(ub);
        }
        if let Some(ref actor) = filter.actor {
            q = q.bind(actor);
        }
        if let Some(ref audience) = filter.audience {
            q = q.bind(audience);
        }

        let result = q
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        Ok(result.rows_affected())
    }

    async fn touch(&self, id: &str) -> Result<(), MemcpError> {
        let now = Utc::now();
        // Silently ignore if id doesn't exist (fire-and-forget)
        let _ = sqlx::query(
            "UPDATE memories SET last_accessed_at = $1, access_count = access_count + 1 WHERE id = $2",
        )
        .bind(&now) // TIMESTAMPTZ — bind DateTime<Utc> directly
        .bind(id)
        .execute(&self.pool)
        .await;

        Ok(())
    }
}

impl PostgresMemoryStore {
    /// Insert a new embedding record for a memory.
    pub async fn insert_embedding(
        &self,
        id: &str,
        memory_id: &str,
        model_name: &str,
        model_version: &str,
        dimension: i32,
        embedding: &pgvector::Vector,
        is_current: bool,
    ) -> Result<(), MemcpError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO memory_embeddings \
             (id, memory_id, model_name, model_version, dimension, embedding, is_current, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(memory_id)
        .bind(model_name)
        .bind(model_version)
        .bind(dimension)
        .bind(embedding)
        .bind(is_current)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to insert embedding: {}", e)))?;

        Ok(())
    }

    /// Update the embedding_status field on a memory (internal metadata — does not update updated_at).
    pub async fn update_embedding_status(
        &self,
        memory_id: &str,
        status: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query("UPDATE memories SET embedding_status = $1 WHERE id = $2")
            .bind(status)
            .bind(memory_id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to update embedding status: {}", e)))?;

        Ok(())
    }

    /// Retrieve memories that need embedding (status 'pending' or 'failed'), ordered oldest first.
    pub async fn get_pending_memories(&self, limit: i64) -> Result<Vec<crate::store::Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience \
             FROM memories WHERE embedding_status IN ('pending', 'failed') AND deleted_at IS NULL \
             ORDER BY created_at ASC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        rows.iter().map(row_to_memory).collect()
    }

    /// Return embedding statistics grouped by status and by model.
    ///
    /// Returns:
    /// ```json
    /// { "by_status": { "pending": N, "complete": N, "failed": N },
    ///   "by_model": [ { "model_name": ..., "model_version": ..., "is_current": true, "count": N } ] }
    /// ```
    pub async fn embedding_stats(&self) -> Result<serde_json::Value, MemcpError> {
        // Query 1: counts by embedding_status (exclude soft-deleted)
        let status_rows = sqlx::query(
            "SELECT embedding_status, COUNT(*) as count FROM memories WHERE deleted_at IS NULL GROUP BY embedding_status",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        let mut by_status = serde_json::Map::new();
        for row in &status_rows {
            let status: String = row
                .try_get("embedding_status")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let count: i64 = row
                .try_get("count")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            by_status.insert(status, serde_json::json!(count));
        }

        // Query 2: counts by model
        let model_rows = sqlx::query(
            "SELECT model_name, model_version, is_current, COUNT(*) as count \
             FROM memory_embeddings GROUP BY model_name, model_version, is_current",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        let mut by_model: Vec<serde_json::Value> = Vec::new();
        for row in &model_rows {
            let model_name: String = row
                .try_get("model_name")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let model_version: String = row
                .try_get("model_version")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let is_current: bool = row
                .try_get("is_current")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let count: i64 = row
                .try_get("count")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            by_model.push(serde_json::json!({
                "model_name": model_name,
                "model_version": model_version,
                "is_current": is_current,
                "count": count,
            }));
        }

        Ok(serde_json::json!({
            "by_status": by_status,
            "by_model": by_model,
        }))
    }

    /// Mark ALL current embeddings as stale (used when switching to a new embedding model).
    ///
    /// Sets is_current = false on all memory_embeddings, and resets embedding_status = 'pending'
    /// on all affected memories so the backfill can re-embed them with the new model.
    /// Returns the count of embeddings marked stale.
    pub async fn mark_all_embeddings_stale(&self) -> Result<u64, MemcpError> {
        // Step 1: mark all current embeddings stale and collect affected memory_ids
        let rows = sqlx::query(
            "UPDATE memory_embeddings SET is_current = false, updated_at = NOW() \
             WHERE is_current = true RETURNING memory_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to mark embeddings stale: {}", e)))?;

        let count = rows.len() as u64;

        if count > 0 {
            // Step 2: collect memory_ids and reset their embedding_status to 'pending'
            let memory_ids: Vec<String> = rows
                .iter()
                .filter_map(|r| r.try_get::<String, _>("memory_id").ok())
                .collect();

            sqlx::query(
                "UPDATE memories SET embedding_status = 'pending' WHERE id = ANY($1)",
            )
            .bind(&memory_ids)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to reset memory embedding_status: {}", e))
            })?;
        }

        Ok(count)
    }

    /// Query the dimension of the most recent current embedding in the database.
    ///
    /// Returns None when no current embeddings exist (fresh DB or after full purge).
    /// Used by `embed switch-model` to detect dimension changes before switching models.
    pub async fn current_embedding_dimension(&self) -> Result<Option<usize>, MemcpError> {
        let result = sqlx::query_scalar::<_, i32>(
            "SELECT dimension FROM memory_embeddings WHERE is_current = true LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to query dimension: {}", e)))?;

        Ok(result.map(|d| d as usize))
    }

    /// Delete ALL embeddings and reset all memories to pending.
    ///
    /// Used when switching to a model with different dimensions — existing embeddings
    /// are incompatible and cannot be compared against new-model embeddings via cosine distance.
    /// Source memories are never deleted — only the derived embedding vectors are removed.
    ///
    /// Returns the count of embedding rows deleted.
    pub async fn purge_all_embeddings(&self) -> Result<u64, MemcpError> {
        let result = sqlx::query("DELETE FROM memory_embeddings")
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to purge embeddings: {}", e)))?;

        let count = result.rows_affected();

        sqlx::query("UPDATE memories SET embedding_status = 'pending'")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to reset embedding status: {}", e))
            })?;

        Ok(count)
    }

    /// Return the underlying PgPool so embedding pipeline can share the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Returns whether the ParadeDB pg_search extension is available on this PostgreSQL instance.
    /// Detected once at construction time — cached for the lifetime of the store.
    pub fn paradedb_available(&self) -> bool {
        self.paradedb_available
    }

    /// Fetch salience rows for a batch of memory IDs from memory_salience table.
    ///
    /// Returns defaults (stability=1.0, difficulty=5.0, count=0) for IDs with no row.
    /// Uses ANY($1) array binding for efficient batch fetch.
    pub async fn get_salience_data(
        &self,
        memory_ids: &[String],
    ) -> Result<HashMap<String, SalienceRow>, MemcpError> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = sqlx::query(
            "SELECT memory_id, stability, difficulty, reinforcement_count, last_reinforced_at \
             FROM memory_salience \
             WHERE memory_id = ANY($1)",
        )
        .bind(memory_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch salience data: {}", e)))?;

        let mut map: HashMap<String, SalienceRow> = HashMap::with_capacity(rows.len());
        for row in &rows {
            let memory_id: String = row
                .try_get("memory_id")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let stability: f64 = row
                .try_get::<f32, _>("stability")
                .map(|v| v as f64)
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let difficulty: f64 = row
                .try_get::<f32, _>("difficulty")
                .map(|v| v as f64)
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let reinforcement_count: i32 = row
                .try_get("reinforcement_count")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            let last_reinforced_at: Option<DateTime<Utc>> = row
                .try_get("last_reinforced_at")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            map.insert(
                memory_id,
                SalienceRow {
                    stability,
                    difficulty,
                    reinforcement_count,
                    last_reinforced_at,
                },
            );
        }

        // Fill defaults for IDs not in the table
        for id in memory_ids {
            map.entry(id.clone()).or_default();
        }

        Ok(map)
    }

    /// Insert or update the salience row for a memory (FSRS state).
    ///
    /// Uses INSERT ON CONFLICT DO UPDATE to handle both create and update atomically.
    pub async fn upsert_salience(
        &self,
        memory_id: &str,
        stability: f64,
        difficulty: f64,
        reinforcement_count: i32,
        last_reinforced_at: Option<DateTime<Utc>>,
    ) -> Result<(), MemcpError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO memory_salience \
             (memory_id, stability, difficulty, reinforcement_count, last_reinforced_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) \
             ON CONFLICT (memory_id) DO UPDATE SET \
               stability = EXCLUDED.stability, \
               difficulty = EXCLUDED.difficulty, \
               reinforcement_count = EXCLUDED.reinforcement_count, \
               last_reinforced_at = EXCLUDED.last_reinforced_at, \
               updated_at = EXCLUDED.updated_at",
        )
        .bind(memory_id)
        .bind(stability as f32)
        .bind(difficulty as f32)
        .bind(reinforcement_count)
        .bind(last_reinforced_at)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to upsert salience: {}", e)))?;

        Ok(())
    }

    /// Explicitly reinforce a memory's salience using an FSRS-inspired stability update.
    ///
    /// The key spaced repetition property (SRCH-04): faded memories (low retrievability)
    /// receive a larger stability boost than fresh memories (high retrievability).
    /// Formula: new_stability = stability * (1.0 + (1.0 - retrievability) * multiplier)
    /// where multiplier=1.5 for "good", 2.0 for "easy".
    ///
    /// Clamps resulting stability to [0.1, 36500.0] (0.1 days to ~100 years).
    /// Increments reinforcement_count and sets last_reinforced_at = now.
    pub async fn reinforce_salience(
        &self,
        memory_id: &str,
        rating: &str,
    ) -> Result<SalienceRow, MemcpError> {
        // 1. Fetch current salience row (defaults if no row exists)
        let row_map = self.get_salience_data(&[memory_id.to_string()]).await?;
        let current = row_map.get(memory_id).cloned().unwrap_or_default();

        // 2. Compute days elapsed since last reinforcement (or 365 if never reinforced)
        let days_elapsed = current.last_reinforced_at
            .map(|dt| {
                let duration = Utc::now().signed_duration_since(dt);
                (duration.num_seconds() as f64 / 86_400.0).max(0.0)
            })
            .unwrap_or(365.0);

        // 3. Compute current retrievability (how fresh is the memory right now?)
        let retrievability = crate::search::salience::fsrs_retrievability(
            current.stability,
            days_elapsed,
        );

        // 4. Update stability — faded memories (low retrievability) get bigger boosts
        //    multiplier: 1.5 for "good", 2.0 for "easy"
        let multiplier = if rating == "easy" { 2.0_f64 } else { 1.5_f64 };
        let new_stability = current.stability * (1.0 + (1.0 - retrievability) * multiplier);

        // 5. Clamp to [0.1, 36500.0]
        let new_stability = new_stability.clamp(0.1, 36_500.0);

        let new_count = current.reinforcement_count + 1;
        let now = Utc::now();

        // 6. Upsert to memory_salience
        sqlx::query(
            "INSERT INTO memory_salience \
             (memory_id, stability, difficulty, reinforcement_count, last_reinforced_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) \
             ON CONFLICT (memory_id) DO UPDATE SET \
               stability = EXCLUDED.stability, \
               reinforcement_count = EXCLUDED.reinforcement_count, \
               last_reinforced_at = EXCLUDED.last_reinforced_at, \
               updated_at = EXCLUDED.updated_at",
        )
        .bind(memory_id)
        .bind(new_stability as f32)
        .bind(current.difficulty as f32)
        .bind(new_count)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to reinforce salience: {}", e)))?;

        // 7. Return updated SalienceRow
        Ok(SalienceRow {
            stability: new_stability,
            difficulty: current.difficulty,
            reinforcement_count: new_count,
            last_reinforced_at: Some(now),
        })
    }

    /// Apply a small implicit salience bump from direct memory retrieval.
    ///
    /// stability *= 1.1 — passive access gently maintains freshness.
    /// Uses INSERT ON CONFLICT for lazy row creation.
    /// Does NOT update last_reinforced_at or increment reinforcement_count.
    pub async fn touch_salience(&self, memory_id: &str) -> Result<(), MemcpError> {
        let sql = "INSERT INTO memory_salience (memory_id, stability, updated_at) \
            VALUES ($1, 1.1, NOW()) \
            ON CONFLICT (memory_id) \
            DO UPDATE SET \
                stability = memory_salience.stability * 1.1, \
                updated_at = NOW()";

        sqlx::query(sql)
            .bind(memory_id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Search for memories semantically similar to the query embedding.
    ///
    /// Uses HNSW approximate nearest neighbor search ordered by cosine distance ascending.
    /// When filters are present, enables hnsw.iterative_scan to prevent over-filtering.
    /// Returns results with similarity scores, total match count, and cursor-based pagination.
    ///
    /// Offset-based pagination (filter.offset > 0) is deprecated — use filter.cursor instead.
    /// A deprecation warning is emitted to tracing when offset > 0 without a cursor.
    pub async fn search_similar(
        &self,
        filter: &SearchFilter,
    ) -> Result<SearchResult, MemcpError> {
        // Deprecation warning: offset-based pagination is superseded by cursor-based.
        if filter.offset > 0 && filter.cursor.is_none() {
            tracing::warn!(
                offset = filter.offset,
                "Offset-based search pagination is deprecated. Use cursor-based pagination \
                 (next_cursor from results). Offset support will be removed in a future version."
            );
        }

        // Acquire an explicit connection — SET hnsw.iterative_scan is session-scoped
        // and must run on the same connection as the search query.
        let mut conn = self.pool.acquire().await.map_err(|e| {
            MemcpError::Storage(format!("Failed to acquire connection: {}", e))
        })?;

        // Determine if any optional filters are present
        let has_filters = filter.created_after.is_some()
            || filter.created_before.is_some()
            || filter.tags.is_some();

        // Enable iterative scan when filters are present to prevent over-filtering.
        // Iterative scan requires pgvector 0.8.0+ — gracefully skip if SET fails.
        if has_filters {
            if let Err(e) = sqlx::query("SET hnsw.iterative_scan = 'relaxed_order'")
                .execute(&mut *conn)
                .await
            {
                tracing::warn!(
                    "Failed to set hnsw.iterative_scan (pgvector < 0.8.0?): {}",
                    e
                );
            }
        }

        // Build WHERE conditions with numbered PostgreSQL parameters.
        // $1 is always the query embedding — build filter params starting at $2.
        let mut conditions: Vec<String> = Vec::new();
        // Always filter for current embeddings on complete memories and exclude soft-deleted
        conditions.push("me.is_current = true".to_string());
        conditions.push("m.embedding_status = 'complete'".to_string());
        conditions.push("m.deleted_at IS NULL".to_string());

        let mut param_idx: u32 = 2; // $1 is reserved for query_embedding

        if filter.created_after.is_some() {
            conditions.push(format!("m.created_at > ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_before.is_some() {
            conditions.push(format!("m.created_at < ${}", param_idx));
            param_idx += 1;
        }
        if filter.tags.is_some() {
            // JSONB containment: matches memories that have ALL specified tags
            conditions.push(format!("m.tags @> ${}::jsonb", param_idx));
            param_idx += 1;
        }
        if filter.audience.is_some() {
            conditions.push(format!("m.audience = ${}", param_idx));
            param_idx += 1;
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));

        // Main search query: JOIN memories with embeddings, compute cosine similarity,
        // ORDER BY distance ASC (NOT alias) so HNSW index is used.
        // Suppress consolidated originals from search results.
        let sql = format!(
            "SELECT m.id, m.content, m.type_hint, m.source, m.tags, \
                    m.created_at, m.updated_at, m.last_accessed_at, \
                    m.access_count, m.embedding_status, \
                    m.extracted_entities, m.extracted_facts, m.extraction_status, \
                    m.is_consolidated_original, m.consolidated_into, \
                    m.actor, m.actor_type, m.audience, \
                    (1 - (me.embedding <=> $1)) AS similarity \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id \
             {} AND m.is_consolidated_original = FALSE \
             ORDER BY me.embedding <=> $1 ASC \
             LIMIT ${} OFFSET ${}",
            where_clause, param_idx, param_idx + 1
        );

        // Count query: same JOIN and WHERE but no ORDER BY / LIMIT / OFFSET
        let count_sql = format!(
            "SELECT COUNT(*) as total \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id \
             {} AND m.is_consolidated_original = FALSE",
            where_clause
        );

        // Helper: bind all optional filter params (same order for both queries)
        // We build the binding in a macro-like closure to avoid code duplication.
        // Binding order: $1=query_embedding, $2=created_after?, $3=created_before?, $4=tags?

        // Execute main search query
        let mut q = sqlx::query(&sql).bind(&filter.query_embedding);
        if let Some(ref ca) = filter.created_after {
            q = q.bind(ca);
        }
        if let Some(ref cb) = filter.created_before {
            q = q.bind(cb);
        }
        if let Some(ref tags) = filter.tags {
            q = q.bind(serde_json::json!(tags));
        }
        if let Some(ref audience) = filter.audience {
            q = q.bind(audience);
        }
        q = q.bind(filter.limit).bind(filter.offset);

        let rows = q
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| MemcpError::Storage(format!("Search query failed: {}", e)))?;

        // Execute count query on same connection
        let mut count_q = sqlx::query(&count_sql).bind(&filter.query_embedding);
        if let Some(ref ca) = filter.created_after {
            count_q = count_q.bind(ca);
        }
        if let Some(ref cb) = filter.created_before {
            count_q = count_q.bind(cb);
        }
        if let Some(ref tags) = filter.tags {
            count_q = count_q.bind(serde_json::json!(tags));
        }
        if let Some(ref audience) = filter.audience {
            count_q = count_q.bind(audience);
        }

        let count_row = count_q
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| MemcpError::Storage(format!("Search count query failed: {}", e)))?;

        let total_matches: i64 = count_row
            .try_get("total")
            .map_err(|e| MemcpError::Storage(e.to_string()))?;
        let total_matches = total_matches as u64;

        // Parse result rows into SearchHit records
        let mut hits = Vec::with_capacity(rows.len());
        for row in &rows {
            let memory = row_to_memory(row)?;
            let raw_similarity: f64 = row
                .try_get("similarity")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;
            // Clamp to [0.0, 1.0] to handle floating point edge cases
            let similarity = raw_similarity.clamp(0.0, 1.0);
            hits.push(SearchHit { memory, similarity });
        }

        // Compute cursor-based pagination.
        // When filter.cursor is set (new keyset path): determine has_more via total_matches.
        // When only offset is set (deprecated path): use offset arithmetic.
        // Always encode next_cursor as a keyset cursor (similarity, id) for forward compat.
        let has_more = if filter.cursor.is_some() {
            // Cursor-based: has_more based on whether we fetched limit+1 (fetch_all sees exact count)
            total_matches as i64 > filter.limit
        } else {
            // Offset-based: has_more if offset+limit < total
            let next_offset = filter.offset + filter.limit;
            next_offset < total_matches as i64
        };
        let next_cursor = if has_more {
            hits.last().map(|hit| encode_search_keyset_cursor(hit.similarity, &hit.memory.id))
        } else {
            None
        };

        Ok(SearchResult {
            hits,
            total_matches,
            next_cursor,
            has_more,
        })
    }

    /// Fetch full Memory objects for a list of IDs.
    ///
    /// Returns a HashMap<id, Memory> for efficient lookup by ID.
    /// IDs not found in the database are simply absent from the result.
    pub async fn get_memories_by_ids(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, Memory>, MemcpError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, \
             last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience \
             FROM memories WHERE id = ANY($1) AND deleted_at IS NULL",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch memories by ids: {}", e)))?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in &rows {
            let memory = row_to_memory(row)?;
            map.insert(memory.id.clone(), memory);
        }
        Ok(map)
    }

    /// Orchestrate hybrid BM25 + vector + symbolic search with three-way RRF fusion.
    ///
    /// All three legs run independently with a candidate pool of 40 results each.
    /// When query_embedding is None (embedding provider unavailable), gracefully
    /// falls back to BM25 + symbolic search only.
    ///
    /// Per-leg k overrides control RRF smoothing (lower k = more top-result influence):
    /// - None means "skip this leg entirely"
    /// - Some(k) means "run with this k value" (default: bm25=60.0, vector=60.0, symbolic=40.0)
    ///
    /// Salience re-ranking is NOT performed here — the server layer applies it
    /// after fetching salience data from the database.
    pub async fn hybrid_search(
        &self,
        query_text: &str,
        query_embedding: Option<&pgvector::Vector>,
        limit: i64,
        created_after: Option<chrono::DateTime<Utc>>,
        created_before: Option<chrono::DateTime<Utc>>,
        tags: Option<&[String]>,
        bm25_k: Option<f64>,
        vector_k: Option<f64>,
        symbolic_k: Option<f64>,
        source: Option<&str>,
        audience: Option<&str>,
    ) -> Result<Vec<crate::search::HybridRawHit>, MemcpError> {
        // 40 candidates per leg — research recommendation balancing recall vs cost
        let candidate_limit = 40i64;

        // BM25 leg — skip when bm25_k is None (weight=0.0 = disabled)
        let bm25_results: Vec<(String, i64)> = if bm25_k.is_some() {
            self.search_bm25(query_text, candidate_limit).await?
        } else {
            tracing::info!("BM25 search leg disabled (bm25_weight=0.0)");
            vec![]
        };

        // Vector leg — only runs when query embedding is available AND vector_k is Some
        let vector_results: Vec<(String, i64)> = if vector_k.is_some() {
            if let Some(embedding) = query_embedding {
                let filter = SearchFilter {
                    query_embedding: embedding.clone(),
                    limit: candidate_limit,
                    offset: 0,
                    cursor: None,
                    created_after,
                    created_before,
                    tags: tags.map(|t| t.to_vec()),
                    audience: audience.map(|s| s.to_string()),
                };
                let result = self.search_similar(&filter).await?;
                result
                    .hits
                    .iter()
                    .enumerate()
                    .map(|(i, hit)| (hit.memory.id.clone(), (i + 1) as i64))
                    .collect()
            } else {
                tracing::info!("No query embedding available — skipping vector search leg");
                vec![]
            }
        } else {
            tracing::info!("Vector search leg disabled (vector_weight=0.0)");
            vec![]
        };

        // Symbolic leg — skip when symbolic_k is None (weight=0.0 = disabled)
        let symbolic_results: Vec<(String, i64)> = if symbolic_k.is_some() {
            self.search_symbolic(query_text, candidate_limit).await?
        } else {
            tracing::info!("Symbolic search leg disabled (symbolic_weight=0.0)");
            vec![]
        };

        // Three-way RRF fusion with per-leg k parameters
        let fused = crate::search::rrf_fuse(
            &bm25_results,
            &vector_results,
            &symbolic_results,
            bm25_k.unwrap_or(60.0),
            vector_k.unwrap_or(60.0),
            symbolic_k.unwrap_or(40.0),
        );

        // Fetch full Memory objects for the top fused IDs
        let top_ids: Vec<String> = fused
            .iter()
            .take(limit as usize)
            .map(|(id, _, _)| id.clone())
            .collect();
        let memories = self.get_memories_by_ids(&top_ids).await?;

        // Build HybridRawHit results, preserving RRF rank order
        let mut hits = Vec::new();
        for (id, rrf_score, match_source) in fused.iter().take(limit as usize) {
            if let Some(memory) = memories.get(id) {
                hits.push(crate::search::HybridRawHit {
                    memory: memory.clone(),
                    rrf_score: *rrf_score,
                    match_source: match_source.clone(),
                });
            }
        }

        // Post-filter fused results by source prefix (e.g. "openclaw" matches "openclaw/vita")
        if let Some(src) = source {
            hits.retain(|hit| hit.memory.source.starts_with(src));
        }

        // Post-filter fused results by audience if specified
        if let Some(aud) = audience {
            hits.retain(|hit| hit.memory.audience == aud);
        }

        Ok(hits)
    }

    /// Cursor-based paginated hybrid search.
    ///
    /// Wraps `hybrid_search` with application-level keyset cursor pagination using (rrf_score, id)
    /// pairs. Fetches a larger candidate pool (limit * 3), sorts by rrf_score DESC, then applies
    /// cursor filtering to produce stable non-overlapping pages.
    ///
    /// Encodes `next_cursor` as a keyset cursor: base64url({"t":"s","s":<rrf_score>,"i":"<id>"})
    ///
    /// SCF-04: Cursor-based pagination for search yields non-overlapping pages.
    #[allow(clippy::too_many_arguments)]
    pub async fn hybrid_search_paged(
        &self,
        query_text: &str,
        query_embedding: Option<&pgvector::Vector>,
        limit: i64,
        cursor: Option<String>,
        created_after: Option<chrono::DateTime<Utc>>,
        created_before: Option<chrono::DateTime<Utc>>,
        tags: Option<&[String]>,
        bm25_k: Option<f64>,
        vector_k: Option<f64>,
        symbolic_k: Option<f64>,
        source: Option<&str>,
        audience: Option<&str>,
    ) -> Result<SearchResult, MemcpError> {
        // Decode cursor if provided — get (last_score, last_id) position
        let cursor_position: Option<(f64, String)> = if let Some(ref c) = cursor {
            Some(decode_search_keyset_cursor(c)?)
        } else {
            None
        };

        // Fetch a larger candidate pool for application-level pagination.
        // When using a cursor, we need additional candidates beyond the current page.
        let candidate_limit = if cursor_position.is_some() {
            limit * 5  // larger pool to find enough results after cursor position
        } else {
            limit * 3  // standard oversampling for first page
        };

        // Get raw hits from hybrid_search
        let raw_hits = self.hybrid_search(
            query_text,
            query_embedding,
            candidate_limit,
            created_after,
            created_before,
            tags,
            bm25_k,
            vector_k,
            symbolic_k,
            source,
            audience,
        ).await?;

        // Sort by rrf_score DESC for stable ordering, then by id ASC for tie-breaking
        let mut sorted_hits: Vec<crate::search::HybridRawHit> = raw_hits;
        sorted_hits.sort_by(|a, b| {
            b.rrf_score.partial_cmp(&a.rrf_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.memory.id.cmp(&b.memory.id))
        });

        // Apply cursor filtering: skip items at or before the cursor position
        // Cursor encodes the LAST item of the previous page (score, id).
        // Skip items where: score > last_score, OR (score == last_score AND id <= last_id).
        let filtered: Vec<crate::search::HybridRawHit> = if let Some((last_score, ref last_id)) = cursor_position {
            sorted_hits.into_iter().filter(|hit| {
                let score = hit.rrf_score;
                if (score - last_score).abs() < f64::EPSILON {
                    // Same score: only include items with id > last_id (tie-breaking by id)
                    hit.memory.id.as_str() > last_id.as_str()
                } else {
                    // Different score: include items with lower rrf_score (come after in DESC order)
                    score < last_score
                }
            }).collect()
        } else {
            sorted_hits
        };

        // Take limit + 1 to detect if there are more results
        let has_more = filtered.len() as i64 > limit;
        let take = if has_more { limit as usize } else { filtered.len() };

        let page: Vec<crate::search::HybridRawHit> = filtered.into_iter().take(take).collect();

        // Build next_cursor from the last item on this page
        let next_cursor = if has_more {
            page.last().map(|hit| encode_search_keyset_cursor(hit.rrf_score, &hit.memory.id))
        } else {
            None
        };

        // Convert to SearchHit (using rrf_score as similarity proxy at the store layer)
        let hits: Vec<SearchHit> = page.into_iter().map(|hit| SearchHit {
            similarity: hit.rrf_score,
            memory: hit.memory,
        }).collect();

        let total_matches = hits.len() as u64 + if has_more { 1 } else { 0 };

        Ok(SearchResult {
            hits,
            total_matches,
            next_cursor,
            has_more,
        })
    }

    /// Search for memories matching query terms against symbolic metadata fields.
    ///
    /// Matches against: tags, extracted_entities, extracted_facts (JSONB containment),
    /// type_hint and source (ILIKE). Results scored by match strength, returned as
    /// (memory_id, symbolic_rank) pairs ordered by rank ascending (1 = best match).
    ///
    /// Suppresses consolidated originals from results (is_consolidated_original = FALSE).
    pub async fn search_symbolic(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, MemcpError> {
        // Build JSONB array for containment matching: ["query term"]
        // This matches tags/entities/facts that contain the query string as an element.
        let query_jsonb = serde_json::json!([query]);
        // ILIKE pattern for type_hint and source matching
        let ilike_pattern = format!("%{}%", query);

        let sql = "SELECT id, ROW_NUMBER() OVER (ORDER BY score DESC) AS symbolic_rank
            FROM (
                SELECT id,
                    (CASE WHEN tags @> $1::jsonb THEN 3 ELSE 0 END
                     + CASE WHEN extracted_entities @> $1::jsonb THEN 2 ELSE 0 END
                     + CASE WHEN extracted_facts @> $1::jsonb THEN 2 ELSE 0 END
                     + CASE WHEN type_hint ILIKE $2 THEN 1 ELSE 0 END
                     + CASE WHEN source ILIKE $2 THEN 1 ELSE 0 END) AS score
                FROM memories
                WHERE is_consolidated_original = FALSE
                  AND deleted_at IS NULL
                  AND (
                    tags @> $1::jsonb
                    OR extracted_entities @> $1::jsonb
                    OR extracted_facts @> $1::jsonb
                    OR type_hint ILIKE $2
                    OR source ILIKE $2
                  )
            ) ranked
            WHERE score > 0
            ORDER BY symbolic_rank
            LIMIT $3";

        let rows = sqlx::query(sql)
            .bind(&query_jsonb)
            .bind(&ilike_pattern)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Symbolic search failed: {}", e)))?;

        rows.iter().map(|row| {
            let id: String = row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))?;
            let rank: i64 = row.try_get("symbolic_rank").map_err(|e| MemcpError::Storage(e.to_string()))?;
            Ok((id, rank))
        }).collect::<Result<Vec<_>, MemcpError>>()
    }

    /// Search for memories matching the query using BM25 full-text ranking.
    ///
    /// Uses native PostgreSQL tsvector/ts_rank_cd by default. When use_paradedb is true
    /// (ParadeDB available AND bm25_backend=paradedb configured), uses pg_search extension
    /// for true BM25 scoring.
    ///
    /// Returns (memory_id, bm25_rank) pairs ordered by relevance. Rank is a 1-based position
    /// (lower = more relevant) for the native path; same semantics for ParadeDB path.
    pub async fn search_bm25(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, MemcpError> {
        let sql = if self.use_paradedb {
            // ParadeDB path: true BM25 scoring via pg_search extension
            // Uses ParadeDB's @@@ operator and paradedb.score() function for BM25 ranking
            "SELECT id, ROW_NUMBER() OVER (
                ORDER BY paradedb.score(id) DESC
            ) AS bm25_rank
            FROM memories
            WHERE content @@@ $1
              AND is_consolidated_original = FALSE
              AND deleted_at IS NULL
            ORDER BY bm25_rank
            LIMIT $2"
        } else {
            // Native PostgreSQL tsvector path — uses GIN index from migration 004
            // ts_rank_cd uses cover density ranking; ORDER BY bm25_rank for result order
            "SELECT id, ROW_NUMBER() OVER (
                ORDER BY ts_rank_cd(
                    to_tsvector('english', content),
                    plainto_tsquery('english', $1)
                ) DESC
            ) AS bm25_rank
            FROM memories
            WHERE to_tsvector('english', content) @@ plainto_tsquery('english', $1)
              AND is_consolidated_original = FALSE
              AND deleted_at IS NULL
            ORDER BY bm25_rank
            LIMIT $2"
        };

        let rows = sqlx::query(sql)
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("BM25 search failed: {}", e)))?;

        rows.iter().map(|row| {
            let id: String = row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))?;
            let rank: i64 = row.try_get("bm25_rank").map_err(|e| MemcpError::Storage(e.to_string()))?;
            Ok((id, rank))
        }).collect::<Result<Vec<_>, MemcpError>>()
    }

    // -------------------------------------------------------------------------
    // Extraction pipeline support methods
    // -------------------------------------------------------------------------

    /// Store extraction results (entities and facts) for a memory.
    ///
    /// Updates the extracted_entities and extracted_facts JSONB columns.
    /// Called by the extraction pipeline after successful entity/fact extraction.
    pub async fn update_extraction_results(
        &self,
        memory_id: &str,
        entities: &[String],
        facts: &[String],
    ) -> Result<(), MemcpError> {
        let entities_json = serde_json::json!(entities);
        let facts_json = serde_json::json!(facts);

        sqlx::query(
            "UPDATE memories SET extracted_entities = $2, extracted_facts = $3 WHERE id = $1",
        )
        .bind(memory_id)
        .bind(&entities_json)
        .bind(&facts_json)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update extraction results: {}", e)))?;

        Ok(())
    }

    /// Update the extraction_status column for a memory.
    ///
    /// Valid statuses: "pending", "complete", "failed".
    pub async fn update_extraction_status(
        &self,
        memory_id: &str,
        status: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query("UPDATE memories SET extraction_status = $2 WHERE id = $1")
            .bind(memory_id)
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to update extraction status: {}", e)))?;

        Ok(())
    }

    /// Fetch memories with pending extraction status for backfill.
    ///
    /// Returns (id, content) pairs for queuing into the extraction pipeline.
    pub async fn get_pending_extraction(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String)>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content FROM memories WHERE extraction_status = 'pending' AND deleted_at IS NULL LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch pending extractions: {}", e)))?;

        rows.iter()
            .map(|row| {
                let id: String = row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))?;
                let content: String = row.try_get("content").map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok((id, content))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    // -------------------------------------------------------------------------
    // Consolidation pipeline support methods
    // -------------------------------------------------------------------------

    /// Atomically create a consolidated memory and link its originals.
    ///
    /// Runs in a single database transaction:
    /// 1. INSERT a new memory row with `type_hint='consolidated'`, `source='consolidation'`.
    /// 2. For each source_id: INSERT into `memory_consolidations` with similarity score.
    /// 3. For each source_id: UPDATE memories SET `is_consolidated_original=TRUE`, `consolidated_into=id`.
    ///
    /// The UNIQUE constraint on (consolidated_id, original_id) prevents race conditions —
    /// concurrent workers attempting the same consolidation will get a duplicate key error,
    /// which the caller should handle gracefully by ignoring the violation.
    ///
    /// Returns the new consolidated memory's ID.
    pub async fn create_consolidated_memory(
        &self,
        content: &str,
        source_ids: &[String],
        similarities: &[f64],
    ) -> Result<String, MemcpError> {
        let consolidated_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // Start a database transaction for atomic create + link + mark
        let mut tx = self.pool.begin().await.map_err(|e| {
            MemcpError::Storage(format!("Failed to begin consolidation transaction: {}", e))
        })?;

        // 1. Insert the consolidated memory row
        sqlx::query(
            "INSERT INTO memories \
             (id, content, type_hint, source, created_at, updated_at, access_count, \
              embedding_status, extraction_status, actor_type, audience) \
             VALUES ($1, $2, 'consolidated', 'consolidation', $3, $3, 0, 'pending', 'pending', 'system', 'global')",
        )
        .bind(&consolidated_id)
        .bind(content)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to insert consolidated memory: {}", e)))?;

        // 2. Insert consolidation provenance records + mark originals
        for (source_id, &similarity) in source_ids.iter().zip(similarities.iter()) {
            let link_id = Uuid::new_v4().to_string();

            // Insert memory_consolidations record
            sqlx::query(
                "INSERT INTO memory_consolidations \
                 (id, consolidated_id, original_id, similarity_score, created_at) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&link_id)
            .bind(&consolidated_id)
            .bind(source_id)
            .bind(similarity as f32)  // REAL column — use f32
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to insert consolidation link: {}", e)))?;

            // Mark original as consolidated
            sqlx::query(
                "UPDATE memories SET is_consolidated_original = TRUE, consolidated_into = $1 \
                 WHERE id = $2",
            )
            .bind(&consolidated_id)
            .bind(source_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to mark original as consolidated: {}", e)))?;
        }

        // Commit the transaction atomically
        tx.commit().await.map_err(|e| {
            MemcpError::Storage(format!("Failed to commit consolidation transaction: {}", e))
        })?;

        Ok(consolidated_id)
    }

    // -------------------------------------------------------------------------
    // GC (garbage collection) store methods
    // -------------------------------------------------------------------------

    /// Count live (non-soft-deleted) memories.
    pub async fn count_live_memories(&self) -> Result<i64, MemcpError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to count live memories: {}", e)))?;
        Ok(count)
    }

    /// Fetch GC candidates: low-salience memories older than min_age_days.
    ///
    /// Excludes consolidated originals (they should never be pruned individually).
    /// Returns candidates sorted by stability ascending (lowest first).
    pub async fn get_gc_candidates(
        &self,
        salience_threshold: f64,
        min_age_days: u32,
        limit: i64,
    ) -> Result<Vec<crate::gc::GcCandidate>, MemcpError> {
        let rows = sqlx::query(
            "SELECT m.id, LEFT(m.content, 100) AS snippet,
                    COALESCE(ms.stability, 1.0)::float8 AS stability,
                    EXTRACT(EPOCH FROM NOW() - m.created_at)::bigint / 86400 AS age_days
             FROM memories m
             LEFT JOIN memory_salience ms ON ms.memory_id = m.id
             WHERE m.deleted_at IS NULL
               AND m.is_consolidated_original = FALSE
               AND COALESCE(ms.stability, 1.0) < $1
               AND m.created_at < NOW() - ($2 || ' days')::interval
             ORDER BY stability ASC
             LIMIT $3",
        )
        .bind(salience_threshold as f32)
        .bind(min_age_days.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch GC candidates: {}", e)))?;

        rows.iter().map(|row| {
            let id: String = row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))?;
            let snippet: String = row.try_get("snippet").map_err(|e| MemcpError::Storage(e.to_string()))?;
            let stability: f64 = row.try_get::<f64, _>("stability").map_err(|e| MemcpError::Storage(e.to_string()))?;
            let age_days: i64 = row.try_get("age_days").map_err(|e| MemcpError::Storage(e.to_string()))?;
            Ok(crate::gc::GcCandidate {
                id,
                content_snippet: snippet,
                stability,
                age_days,
            })
        }).collect::<Result<Vec<_>, MemcpError>>()
    }

    /// Fetch IDs of TTL-expired memories (expires_at < NOW(), not yet soft-deleted).
    pub async fn get_expired_memories(&self) -> Result<Vec<String>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id FROM memories
             WHERE deleted_at IS NULL
               AND expires_at IS NOT NULL
               AND expires_at < NOW()",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch expired memories: {}", e)))?;

        rows.iter().map(|row| {
            row.try_get("id").map_err(|e| MemcpError::Storage(e.to_string()))
        }).collect::<Result<Vec<_>, MemcpError>>()
    }

    /// Soft-delete a batch of memories by setting deleted_at = NOW().
    ///
    /// Returns the number of rows actually updated (may be less than ids.len()
    /// if some were already deleted).
    pub async fn soft_delete_memories(&self, ids: &[String]) -> Result<usize, MemcpError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            "UPDATE memories SET deleted_at = NOW()
             WHERE id = ANY($1) AND deleted_at IS NULL",
        )
        .bind(ids)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to soft-delete memories: {}", e)))?;

        Ok(result.rows_affected() as usize)
    }

    /// Hard purge memories soft-deleted more than grace_days ago.
    ///
    /// Also removes associated rows from memory_embeddings, memory_salience,
    /// extracted_facts (via extracted_facts column — no separate table), and
    /// memory_consolidations.
    ///
    /// Returns the number of memory rows hard-deleted.
    pub async fn hard_purge_old_deleted(&self, grace_days: u32) -> Result<usize, MemcpError> {
        // Collect IDs to purge
        let rows = sqlx::query(
            "SELECT id FROM memories
             WHERE deleted_at IS NOT NULL
               AND deleted_at < NOW() - ($1 || ' days')::interval",
        )
        .bind(grace_days.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch purgeable memories: {}", e)))?;

        if rows.is_empty() {
            return Ok(0);
        }

        let ids: Vec<String> = rows.iter()
            .filter_map(|r| r.try_get::<String, _>("id").ok())
            .collect();

        // Delete dependent rows first
        sqlx::query("DELETE FROM memory_embeddings WHERE memory_id = ANY($1)")
            .bind(&ids)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to purge embeddings: {}", e)))?;

        sqlx::query("DELETE FROM memory_salience WHERE memory_id = ANY($1)")
            .bind(&ids)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to purge salience rows: {}", e)))?;

        sqlx::query("DELETE FROM memory_consolidations WHERE original_id = ANY($1) OR consolidated_id = ANY($1)")
            .bind(&ids)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to purge consolidation rows: {}", e)))?;

        // Finally delete the memories themselves
        let result = sqlx::query("DELETE FROM memories WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to hard purge memories: {}", e)))?;

        Ok(result.rows_affected() as usize)
    }

    /// Update GC metrics in daemon_status after a GC run.
    pub async fn update_gc_metrics(&self, pruned: i64) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE daemon_status SET last_gc_at = NOW(),
             gc_pruned_total = COALESCE(gc_pruned_total, 0) + $1
             WHERE id = 1",
        )
        .bind(pruned)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update GC metrics: {}", e)))?;
        Ok(())
    }

    /// Merge a near-duplicate memory into an existing one.
    ///
    /// Executed in a transaction:
    ///   1. Updates the existing memory: increments access_count, updates last_accessed_at,
    ///      and appends a source entry to dedup_sources JSONB.
    ///   2. Soft-deletes the new (incoming) memory by setting deleted_at = NOW().
    ///
    /// Fail-open callers: if this returns Err, the caller logs and continues (no data loss).
    pub async fn merge_duplicate(
        &self,
        existing_id: &str,
        new_id: &str,
        source: &str,
    ) -> Result<(), MemcpError> {
        let merged_at = Utc::now().to_rfc3339();
        let source_entry = serde_json::json!({
            "source": source,
            "merged_at": merged_at,
            "merged_id": new_id
        });
        let source_jsonb = serde_json::to_string(&source_entry)
            .map_err(|e| MemcpError::Storage(format!("Failed to serialize dedup source: {}", e)))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to begin dedup transaction: {}", e)))?;

        // Update existing memory: bump access_count, refresh last_accessed_at, append to dedup_sources
        sqlx::query(
            "UPDATE memories SET
                 access_count = access_count + 1,
                 last_accessed_at = NOW(),
                 dedup_sources = dedup_sources || $2::jsonb
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(existing_id)
        .bind(&source_jsonb)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to update existing memory in dedup merge: {}", e))
        })?;

        // Soft-delete the incoming duplicate
        sqlx::query("UPDATE memories SET deleted_at = NOW() WHERE id = $1")
            .bind(new_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to soft-delete duplicate in dedup merge: {}", e))
            })?;

        tx.commit()
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to commit dedup merge transaction: {}", e)))?;

        Ok(())
    }

    /// Increment the dedup merge counter in daemon_status.
    ///
    /// Called after each successful dedup merge. Fail-open: if this fails,
    /// the merge still succeeded — only the metric count is affected.
    pub async fn increment_dedup_merges(&self) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE daemon_status SET
                 gc_dedup_merges = COALESCE(gc_dedup_merges, 0) + 1
             WHERE id = 1",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to increment dedup merges: {}", e)))?;
        Ok(())
    }

    /// Apply explicit relevance feedback to a memory's FSRS salience state.
    ///
    /// "useful" maps to a good review: increases stability (multiplier 1.5) and
    /// decreases difficulty (multiplier 0.9 — easier next time).
    ///
    /// "irrelevant" maps to a failed review: sharply decreases stability (multiplier 0.2)
    /// and increases difficulty (multiplier 1.2 — harder next time).
    ///
    /// Fire-and-forget: returns Ok(()) on success, no salience details returned.
    /// This is intentionally a separate method from reinforce_salience — the semantics
    /// ("useful/irrelevant" explicit feedback) are different from FSRS ratings.
    pub async fn apply_feedback(&self, memory_id: &str, signal: &str) -> Result<(), MemcpError> {
        // Validate signal
        if signal != "useful" && signal != "irrelevant" {
            return Err(MemcpError::Validation {
                message: format!(
                    "Invalid feedback signal '{}'. Must be 'useful' or 'irrelevant'.",
                    signal
                ),
                field: Some("signal".to_string()),
            });
        }

        // Fetch current salience row (defaults if no row exists)
        let row_map = self.get_salience_data(&[memory_id.to_string()]).await?;
        let current = row_map.get(memory_id).cloned().unwrap_or_default();

        let (new_stability, new_difficulty) = if signal == "useful" {
            // Good review: increase stability, decrease difficulty
            let stability = (current.stability * 1.5).clamp(0.1, 36_500.0);
            let difficulty = (current.difficulty * 0.9).clamp(0.1, 10.0);
            (stability, difficulty)
        } else {
            // Failed review (irrelevant): sharp stability drop, increase difficulty
            let stability = (current.stability * 0.2).clamp(0.1, 36_500.0);
            let difficulty = (current.difficulty * 1.2).clamp(0.1, 10.0);
            (stability, difficulty)
        };

        let now = Utc::now();

        // Upsert into memory_salience (same pattern as reinforce_salience)
        sqlx::query(
            "INSERT INTO memory_salience \
             (memory_id, stability, difficulty, reinforcement_count, last_reinforced_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) \
             ON CONFLICT (memory_id) DO UPDATE SET \
               stability = EXCLUDED.stability, \
               difficulty = EXCLUDED.difficulty, \
               last_reinforced_at = EXCLUDED.last_reinforced_at, \
               updated_at = EXCLUDED.updated_at",
        )
        .bind(memory_id)
        .bind(new_stability as f32)
        .bind(new_difficulty as f32)
        .bind(current.reinforcement_count)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to apply feedback: {}", e)))?;

        Ok(())
    }

    /// Fetch the current embedding vector for a memory.
    ///
    /// Returns None if no current embedding exists (not yet embedded, or embedding was staled).
    pub async fn get_memory_embedding(
        &self,
        memory_id: &str,
    ) -> Result<Option<pgvector::Vector>, MemcpError> {
        let row = sqlx::query(
            "SELECT embedding FROM memory_embeddings WHERE memory_id = $1 AND is_current = TRUE",
        )
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch memory embedding: {}", e)))?;

        match row {
            None => Ok(None),
            Some(r) => {
                let embedding: pgvector::Vector = r
                    .try_get("embedding")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok(Some(embedding))
            }
        }
    }
}
