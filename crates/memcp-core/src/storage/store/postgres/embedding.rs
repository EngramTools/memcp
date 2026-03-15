#![allow(clippy::unwrap_used)]
//! Embedding operations and vector search for PostgresMemoryStore.

use chrono::Utc;
use sqlx::{postgres::PgPool, Row};
use std::collections::HashMap;

use super::{row_to_memory, PostgresMemoryStore};
use crate::errors::MemcpError;
use crate::store::{
    decode_search_keyset_cursor, encode_search_keyset_cursor, Memory, SearchFilter, SearchHit,
    SearchResult,
};

impl PostgresMemoryStore {
    /// Insert a new embedding record for a memory.
    ///
    /// The `tier` parameter identifies which embedding tier this belongs to
    /// (e.g., "fast" for local model, "quality" for API model).
    #[allow(clippy::too_many_arguments)] // All params are semantically distinct; struct refactor deferred
    pub async fn insert_embedding(
        &self,
        id: &str,
        memory_id: &str,
        model_name: &str,
        model_version: &str,
        dimension: i32,
        embedding: &pgvector::Vector,
        is_current: bool,
        tier: &str,
    ) -> Result<(), MemcpError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO memory_embeddings \
             (id, memory_id, model_name, model_version, dimension, embedding, is_current, created_at, updated_at, tier) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(memory_id)
        .bind(model_name)
        .bind(model_version)
        .bind(dimension)
        .bind(embedding)
        .bind(is_current)
        .bind(now)
        .bind(now)
        .bind(tier)
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
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to update embedding status: {}", e))
            })?;

        Ok(())
    }

    /// Retrieve memories that need embedding (status 'pending' or 'failed'), ordered oldest first.
    pub async fn get_pending_memories(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::store::Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
             FROM memories WHERE embedding_status IN ('pending', 'failed') AND deleted_at IS NULL \
             AND abstraction_status != 'pending' \
             ORDER BY created_at ASC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        rows.iter().map(row_to_memory).collect()
    }

    /// Get recent memories that haven't been enriched yet (no 'enriched' tag).
    ///
    /// Used by the enrichment daemon worker to find candidates for retroactive
    /// neighbor-based tag enrichment. Returns the most recently created memories first,
    /// so new memories get enriched before older ones.
    pub async fn get_unenriched_memories(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::store::Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
             FROM memories \
             WHERE deleted_at IS NULL \
               AND embedding_status = 'complete' \
               AND NOT (tags @> '[\"enriched\"]'::jsonb) \
             ORDER BY created_at DESC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch unenriched memories: {}", e)))?;

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

            sqlx::query("UPDATE memories SET embedding_status = 'pending' WHERE id = ANY($1)")
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
            .map_err(|e| MemcpError::Storage(format!("Failed to reset embedding status: {}", e)))?;

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

    /// Search for memories semantically similar to the query embedding.
    ///
    /// Uses HNSW approximate nearest neighbor search ordered by cosine distance ascending.
    /// When filters are present, enables hnsw.iterative_scan to prevent over-filtering.
    /// Returns results with similarity scores, total match count, and cursor-based pagination.
    ///
    /// Offset-based pagination (filter.offset > 0) is deprecated — use filter.cursor instead.
    /// A deprecation warning is emitted to tracing when offset > 0 without a cursor.
    pub async fn search_similar(&self, filter: &SearchFilter) -> Result<SearchResult, MemcpError> {
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
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to acquire connection: {}", e)))?;

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
        // Always filter for current embeddings on complete memories and exclude soft-deleted
        let mut conditions: Vec<String> = vec![
            "me.is_current = true".to_string(),
            "m.embedding_status = 'complete'".to_string(),
            "m.deleted_at IS NULL".to_string(),
            "(m.tags IS NULL OR NOT (m.tags @> '[\"suspicious\"]'::jsonb))".to_string(),
        ];

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
                    m.parent_id, m.chunk_index, m.total_chunks, \
                    m.event_time, m.event_time_precision, m.project, \
                    m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
                    m.abstract_text, m.overview_text, m.abstraction_status, \
                    (1 - (me.embedding <=> $1)) AS similarity \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id \
             {} AND m.is_consolidated_original = FALSE \
             ORDER BY me.embedding <=> $1 ASC \
             LIMIT ${} OFFSET ${}",
            where_clause,
            param_idx,
            param_idx + 1
        );

        // Count query: same JOIN and WHERE but no ORDER BY / LIMIT / OFFSET
        let count_sql = format!(
            "SELECT COUNT(*) as total \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id \
             {} AND m.is_consolidated_original = FALSE",
            where_clause
        );

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
        let has_more = if filter.cursor.is_some() {
            total_matches as i64 > filter.limit
        } else {
            let next_offset = filter.offset + filter.limit;
            next_offset < total_matches as i64
        };
        let next_cursor = if has_more {
            hits.last()
                .map(|hit| encode_search_keyset_cursor(hit.similarity, &hit.memory.id))
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
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
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
    #[allow(clippy::too_many_arguments)]
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
        source: Option<&[String]>,
        audience: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<crate::search::HybridRawHit>, MemcpError> {
        let candidate_limit = 40i64;

        let bm25_results: Vec<(String, i64)> = if bm25_k.is_some() {
            self.search_bm25(query_text, candidate_limit).await?
        } else {
            tracing::info!("BM25 search leg disabled (bm25_weight=0.0)");
            vec![]
        };

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

        let symbolic_results: Vec<(String, i64)> = if symbolic_k.is_some() {
            self.search_symbolic(query_text, candidate_limit).await?
        } else {
            tracing::info!("Symbolic search leg disabled (symbolic_weight=0.0)");
            vec![]
        };

        let fused = crate::search::rrf_fuse(
            &bm25_results,
            &vector_results,
            &symbolic_results,
            bm25_k.unwrap_or(60.0),
            vector_k.unwrap_or(60.0),
            symbolic_k.unwrap_or(40.0),
        );

        let top_ids: Vec<String> = fused
            .iter()
            .take(limit as usize)
            .map(|(id, _, _)| id.clone())
            .collect();
        let memories = self.get_memories_by_ids(&top_ids).await?;

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

        if let Some(sources) = source {
            if !sources.is_empty() {
                hits.retain(|hit| {
                    sources
                        .iter()
                        .any(|src| hit.memory.source.starts_with(src.as_str()))
                });
            }
        }

        if let Some(aud) = audience {
            hits.retain(|hit| hit.memory.audience == aud);
        }

        if let Some(ws) = project {
            hits.retain(|hit| {
                hit.memory.project.as_deref() == Some(ws) || hit.memory.project.is_none()
            });
        }

        Ok(hits)
    }

    /// Vector search within a specific embedding tier.
    ///
    /// Returns `(memory_id, rank)` pairs sorted by cosine similarity.
    /// Uses the tier-specific partial HNSW index for efficient lookup.
    #[allow(clippy::too_many_arguments)]
    async fn search_vector_for_tier(
        &self,
        query_embedding: &pgvector::Vector,
        tier: &str,
        limit: i64,
        created_after: Option<chrono::DateTime<Utc>>,
        created_before: Option<chrono::DateTime<Utc>>,
        tags: Option<&[String]>,
        audience: Option<&str>,
    ) -> Result<Vec<(String, i64)>, MemcpError> {
        let mut conditions: Vec<String> = vec![
            "me.is_current = true".to_string(),
            "me.tier = $2".to_string(),
            "m.embedding_status = 'complete'".to_string(),
            "m.deleted_at IS NULL".to_string(),
            "m.is_consolidated_original = FALSE".to_string(),
            "(m.tags IS NULL OR NOT (m.tags @> '[\"suspicious\"]'::jsonb))".to_string(),
        ];

        let mut param_idx: u32 = 3;

        if created_after.is_some() {
            conditions.push(format!("m.created_at > ${}", param_idx));
            param_idx += 1;
        }
        if created_before.is_some() {
            conditions.push(format!("m.created_at < ${}", param_idx));
            param_idx += 1;
        }
        if tags.is_some() {
            conditions.push(format!("m.tags @> ${}::jsonb", param_idx));
            param_idx += 1;
        }
        if audience.is_some() {
            conditions.push(format!("m.audience = ${}", param_idx));
            param_idx += 1;
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));

        let sql = format!(
            "SELECT m.id \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id \
             {} \
             ORDER BY me.embedding <=> $1 ASC \
             LIMIT ${}",
            where_clause, param_idx
        );

        let mut q = sqlx::query_scalar::<_, String>(&sql)
            .bind(query_embedding)
            .bind(tier);

        if let Some(ca) = created_after {
            q = q.bind(ca);
        }
        if let Some(cb) = created_before {
            q = q.bind(cb);
        }
        if let Some(t) = tags {
            q = q.bind(serde_json::json!(t));
        }
        if let Some(aud) = audience {
            q = q.bind(aud);
        }
        q = q.bind(limit);

        let ids = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Tier vector search failed: {}", e)))?;

        Ok(ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, (i + 1) as i64))
            .collect())
    }

    /// Multi-tier hybrid search: runs BM25 + symbolic once, vector search per tier, then RRF-merges.
    #[allow(clippy::too_many_arguments)]
    pub async fn hybrid_search_multi_tier(
        &self,
        query_text: &str,
        tier_embeddings: &HashMap<String, pgvector::Vector>,
        limit: i64,
        created_after: Option<chrono::DateTime<Utc>>,
        created_before: Option<chrono::DateTime<Utc>>,
        tags: Option<&[String]>,
        bm25_k: Option<f64>,
        vector_k: Option<f64>,
        symbolic_k: Option<f64>,
        source: Option<&[String]>,
        audience: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<crate::search::HybridRawHit>, MemcpError> {
        let candidate_limit = 40i64;

        let bm25_results: Vec<(String, i64)> = if bm25_k.is_some() {
            self.search_bm25(query_text, candidate_limit).await?
        } else {
            vec![]
        };

        let mut all_vector_results: Vec<(String, i64)> = Vec::new();
        if vector_k.is_some() {
            for (tier_name, embedding) in tier_embeddings {
                let tier_results = self
                    .search_vector_for_tier(
                        embedding,
                        tier_name,
                        candidate_limit,
                        created_after,
                        created_before,
                        tags,
                        audience,
                    )
                    .await?;

                for (id, rank) in tier_results {
                    if let Some(existing) =
                        all_vector_results.iter_mut().find(|(eid, _)| eid == &id)
                    {
                        if rank < existing.1 {
                            existing.1 = rank;
                        }
                    } else {
                        all_vector_results.push((id, rank));
                    }
                }
            }
        }

        let symbolic_results: Vec<(String, i64)> = if symbolic_k.is_some() {
            self.search_symbolic(query_text, candidate_limit).await?
        } else {
            vec![]
        };

        let fused = crate::search::rrf_fuse(
            &bm25_results,
            &all_vector_results,
            &symbolic_results,
            bm25_k.unwrap_or(60.0),
            vector_k.unwrap_or(60.0),
            symbolic_k.unwrap_or(40.0),
        );

        let top_ids: Vec<String> = fused
            .iter()
            .take(limit as usize)
            .map(|(id, _, _)| id.clone())
            .collect();
        let memories = self.get_memories_by_ids(&top_ids).await?;

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

        if let Some(sources) = source {
            if !sources.is_empty() {
                hits.retain(|hit| {
                    sources
                        .iter()
                        .any(|src| hit.memory.source.starts_with(src.as_str()))
                });
            }
        }

        if let Some(aud) = audience {
            hits.retain(|hit| hit.memory.audience == aud);
        }

        if let Some(ws) = project {
            hits.retain(|hit| {
                hit.memory.project.as_deref() == Some(ws) || hit.memory.project.is_none()
            });
        }

        Ok(hits)
    }

    /// Cursor-based paginated hybrid search.
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
        source: Option<&[String]>,
        audience: Option<&str>,
        project: Option<&str>,
    ) -> Result<SearchResult, MemcpError> {
        let cursor_position: Option<(f64, String)> = if let Some(ref c) = cursor {
            Some(decode_search_keyset_cursor(c)?)
        } else {
            None
        };

        let candidate_limit = if cursor_position.is_some() {
            limit * 5
        } else {
            limit * 3
        };

        let raw_hits = self
            .hybrid_search(
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
                project,
            )
            .await?;

        let mut sorted_hits: Vec<crate::search::HybridRawHit> = raw_hits;
        sorted_hits.sort_by(|a, b| {
            b.rrf_score
                .partial_cmp(&a.rrf_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.memory.id.cmp(&b.memory.id))
        });

        let filtered: Vec<crate::search::HybridRawHit> =
            if let Some((last_score, ref last_id)) = cursor_position {
                sorted_hits
                    .into_iter()
                    .filter(|hit| {
                        let score = hit.rrf_score;
                        if (score - last_score).abs() < f64::EPSILON {
                            hit.memory.id.as_str() > last_id.as_str()
                        } else {
                            score < last_score
                        }
                    })
                    .collect()
            } else {
                sorted_hits
            };

        let has_more = filtered.len() as i64 > limit;
        let take = if has_more {
            limit as usize
        } else {
            filtered.len()
        };

        let page: Vec<crate::search::HybridRawHit> = filtered.into_iter().take(take).collect();

        let next_cursor = if has_more {
            page.last()
                .map(|hit| encode_search_keyset_cursor(hit.rrf_score, &hit.memory.id))
        } else {
            None
        };

        let hits: Vec<SearchHit> = page
            .into_iter()
            .map(|hit| SearchHit {
                similarity: hit.rrf_score,
                memory: hit.memory,
            })
            .collect();

        let total_matches = hits.len() as u64 + if has_more { 1 } else { 0 };

        Ok(SearchResult {
            hits,
            total_matches,
            next_cursor,
            has_more,
        })
    }

    /// Search for memories matching query terms against symbolic metadata fields.
    pub async fn search_symbolic(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, MemcpError> {
        let query_jsonb = serde_json::json!([query]);
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
                  AND (tags IS NULL OR NOT (tags @> '[\"suspicious\"]'::jsonb))
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

        rows.iter()
            .map(|row| {
                let id: String = row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let rank: i64 = row
                    .try_get("symbolic_rank")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok((id, rank))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    /// Search for memories matching the query using BM25 full-text ranking.
    pub async fn search_bm25(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, MemcpError> {
        let sql = if self.use_paradedb {
            "SELECT id, ROW_NUMBER() OVER (
                ORDER BY paradedb.score(id) DESC
            ) AS bm25_rank
            FROM memories
            WHERE content @@@ $1
              AND is_consolidated_original = FALSE
              AND deleted_at IS NULL
              AND (tags IS NULL OR NOT (tags @> '[\"suspicious\"]'::jsonb))
            ORDER BY bm25_rank
            LIMIT $2"
        } else {
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
              AND (tags IS NULL OR NOT (tags @> '[\"suspicious\"]'::jsonb))
            ORDER BY bm25_rank
            LIMIT $2"
        };

        let rows = sqlx::query(sql)
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("BM25 search failed: {}", e)))?;

        rows.iter()
            .map(|row| {
                let id: String = row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let rank: i64 = row
                    .try_get("bm25_rank")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok((id, rank))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
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

    /// Find memories in the cosine similarity "sweet spot" — related enough to
    /// be meaningful but different enough to be surprising.
    pub async fn discover_associations(
        &self,
        query_embedding: &pgvector::Vector,
        min_similarity: f64,
        max_similarity: f64,
        limit: u32,
        project: Option<&str>,
    ) -> Result<Vec<(Memory, f64)>, MemcpError> {
        let fetch_limit = ((limit as i64) * 3).min(300);

        let sql = if project.is_some() {
            "SELECT m.id, m.content, m.type_hint, m.source, m.tags, \
                    m.created_at, m.updated_at, m.last_accessed_at, \
                    m.access_count, m.embedding_status, \
                    m.extracted_entities, m.extracted_facts, m.extraction_status, \
                    m.is_consolidated_original, m.consolidated_into, \
                    m.actor, m.actor_type, m.audience, \
                    m.parent_id, m.chunk_index, m.total_chunks, \
                    m.event_time, m.event_time_precision, m.project, \
                    m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
                    m.abstract_text, m.overview_text, m.abstraction_status, \
                    (1.0 - (me.embedding <=> $1)) AS similarity \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id AND me.is_current = TRUE \
             WHERE m.deleted_at IS NULL \
               AND m.embedding_status = 'complete' \
               AND (m.project = $3 OR m.project IS NULL) \
             ORDER BY me.embedding <=> $1 ASC \
             LIMIT $2"
                .to_string()
        } else {
            "SELECT m.id, m.content, m.type_hint, m.source, m.tags, \
                    m.created_at, m.updated_at, m.last_accessed_at, \
                    m.access_count, m.embedding_status, \
                    m.extracted_entities, m.extracted_facts, m.extraction_status, \
                    m.is_consolidated_original, m.consolidated_into, \
                    m.actor, m.actor_type, m.audience, \
                    m.parent_id, m.chunk_index, m.total_chunks, \
                    m.event_time, m.event_time_precision, m.project, \
                    m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
                    m.abstract_text, m.overview_text, m.abstraction_status, \
                    (1.0 - (me.embedding <=> $1)) AS similarity \
             FROM memories m \
             JOIN memory_embeddings me ON me.memory_id = m.id AND me.is_current = TRUE \
             WHERE m.deleted_at IS NULL \
               AND m.embedding_status = 'complete' \
             ORDER BY me.embedding <=> $1 ASC \
             LIMIT $2"
                .to_string()
        };

        let rows = if let Some(proj) = project {
            sqlx::query(&sql)
                .bind(query_embedding)
                .bind(fetch_limit)
                .bind(proj)
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query(&sql)
                .bind(query_embedding)
                .bind(fetch_limit)
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| MemcpError::Storage(format!("discover_associations query failed: {}", e)))?;

        let mut results: Vec<(Memory, f64)> = Vec::new();
        for row in &rows {
            let similarity: f64 = row
                .try_get("similarity")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;

            if similarity >= min_similarity && similarity <= max_similarity {
                let memory = row_to_memory(row)?;
                results.push((memory, similarity));
            }
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit as usize);

        Ok(results)
    }
}
