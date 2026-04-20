#![allow(clippy::unwrap_used)]
//! Content extraction, enrichment, GC, curation, sessions, and misc operations
//! for PostgresMemoryStore.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::{
    row_to_memory, CurationActionRow, CurationRunRow, PostgresMemoryStore, RelatedContext,
    SalienceRow,
};
use crate::errors::MemcpError;

impl PostgresMemoryStore {
    // -------------------------------------------------------------------------
    // Extraction pipeline support methods
    // -------------------------------------------------------------------------

    /// Store extraction results (entities and facts) for a memory.
    ///
    /// Updates the extracted_entities and extracted_facts JSONB columns.
    /// When `structured_facts` is non-empty, `extracted_facts` is written as an array
    /// of structured-fact objects so the normalization worker can link facts to entities.
    /// When `structured_facts` is empty, `extracted_facts` is written as the flat
    /// `Vec<String>` for backward compatibility.
    /// Called by the extraction pipeline after successful entity/fact extraction.
    pub async fn update_extraction_results(
        &self,
        memory_id: &str,
        entities: &[String],
        facts: &[String],
        structured_facts: &[crate::pipeline::extraction::StructuredFact],
    ) -> Result<(), MemcpError> {
        let entities_json = serde_json::json!(entities);
        let facts_json = if structured_facts.is_empty() {
            serde_json::json!(facts)
        } else {
            serde_json::json!(structured_facts)
        };

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
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to update extraction status: {}", e))
            })?;

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
                let id: String = row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let content: String = row
                    .try_get("content")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
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
        .bind(now)
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
            .bind(similarity as f32) // REAL column — use f32
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to insert consolidation link: {}", e))
            })?;

            // Mark original as consolidated
            sqlx::query(
                "UPDATE memories SET is_consolidated_original = TRUE, consolidated_into = $1 \
                 WHERE id = $2",
            )
            .bind(&consolidated_id)
            .bind(source_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to mark original as consolidated: {}", e))
            })?;
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
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    MemcpError::Storage(format!("Failed to count live memories: {}", e))
                })?;
        Ok(count)
    }

    /// Count memories with embedding_status = 'pending' (excludes soft-deleted).
    ///
    /// Used by /status endpoint and observability metrics to track embedding backlog.
    pub async fn count_pending_embeddings(&self) -> Result<i64, MemcpError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM memories WHERE embedding_status = 'pending' AND deleted_at IS NULL"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to count pending embeddings: {}", e)))?;
        Ok(count.0)
    }

    /// Count memories with abstraction_status = 'pending' (excludes soft-deleted).
    ///
    /// Used by /status endpoint and observability metrics to track abstraction backlog.
    pub async fn count_pending_abstractions(&self) -> Result<i64, MemcpError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM memories WHERE abstraction_status = 'pending' AND deleted_at IS NULL"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to count pending abstractions: {}", e)))?;
        Ok(count.0)
    }

    /// Fetch memories with abstraction_status = 'pending' for worker processing.
    ///
    /// Returns full Memory rows ordered by creation time (oldest first).
    /// The abstraction worker uses this to process memories that need L0/L1 generation.
    pub async fn get_pending_abstractions(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::store::Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status, knowledge_tier, source_ids \
             FROM memories WHERE abstraction_status = 'pending' AND deleted_at IS NULL \
             ORDER BY created_at ASC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch pending abstractions: {}", e)))?;

        rows.iter().map(row_to_memory).collect()
    }

    /// Update abstract_text, overview_text, and abstraction_status for a memory.
    ///
    /// Called by the abstraction worker on successful LLM generation.
    /// Sets abstraction_status = 'complete' along with the generated texts.
    pub async fn update_abstraction_fields(
        &self,
        id: &str,
        abstract_text: &str,
        overview_text: Option<&str>,
        status: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE memories SET abstract_text = $1, overview_text = $2, abstraction_status = $3, updated_at = NOW() \
             WHERE id = $4",
        )
        .bind(abstract_text)
        .bind(overview_text)
        .bind(status)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update abstraction fields: {}", e)))?;
        Ok(())
    }

    /// Update only abstraction_status for a memory (used on failure).
    ///
    /// Called by the abstraction worker when LLM generation fails.
    /// Fail-open: memory stays usable with full content for embedding.
    pub async fn update_abstraction_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE memories SET abstraction_status = $1, updated_at = NOW() WHERE id = $2",
        )
        .bind(status)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update abstraction status: {}", e)))?;
        Ok(())
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

        rows.iter()
            .map(|row| {
                let id: String = row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let snippet: String = row
                    .try_get("snippet")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let stability: f64 = row
                    .try_get::<f64, _>("stability")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                let age_days: i64 = row
                    .try_get("age_days")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok(crate::gc::GcCandidate {
                    id,
                    content_snippet: snippet,
                    stability,
                    age_days,
                })
            })
            .collect::<Result<Vec<_>, MemcpError>>()
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

        rows.iter()
            .map(|row| {
                row.try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
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

    /// Soft-delete all chunks whose parent_id is in the given set.
    ///
    /// Used by the GC worker to cascade soft-deletes to chunks when a parent
    /// memory is garbage-collected. FK ON DELETE CASCADE only triggers on
    /// hard-delete (DELETE), not on UPDATE of deleted_at.
    pub async fn soft_delete_chunks_by_parents(
        &self,
        parent_ids: &[String],
    ) -> Result<usize, MemcpError> {
        if parent_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            "UPDATE memories SET deleted_at = NOW()
             WHERE parent_id = ANY($1) AND deleted_at IS NULL",
        )
        .bind(parent_ids)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to soft-delete chunks by parent: {}", e))
        })?;

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

        let ids: Vec<String> = rows
            .iter()
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

    /// Delete expired idempotency keys (expires_at < NOW()).
    ///
    /// Called from the GC worker on the same schedule as memory pruning.
    /// Returns the number of expired keys removed.
    pub async fn cleanup_expired_idempotency_keys(&self) -> Result<usize, MemcpError> {
        let result = sqlx::query("DELETE FROM idempotency_keys WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!(
                    "Failed to clean up expired idempotency keys: {}",
                    e
                ))
            })?;
        Ok(result.rows_affected() as usize)
    }

    // -------------------------------------------------------------------------
    // Session and recall management (migration 014)
    // -------------------------------------------------------------------------

    /// Ensure a session exists, updating last_active_at on each call.
    ///
    /// Uses INSERT ... ON CONFLICT DO UPDATE to atomically create-or-touch the session.
    /// This satisfies the FK constraint required by insert_session_recall.
    pub async fn ensure_session(&self, session_id: &str) -> Result<(), MemcpError> {
        sqlx::query(
            "INSERT INTO sessions (session_id, created_at, last_active_at) \
             VALUES ($1, NOW(), NOW()) \
             ON CONFLICT (session_id) DO UPDATE SET last_active_at = NOW()",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to ensure session: {}", e)))?;
        Ok(())
    }

    /// Record a memory as recalled in the current session.
    ///
    /// Calls ensure_session first to satisfy the FK constraint on session_recalls.
    /// Uses ON CONFLICT DO NOTHING — safe to call multiple times for the same
    /// (session_id, memory_id) pair without bumping the recall count.
    pub async fn insert_session_recall(
        &self,
        session_id: &str,
        memory_id: &str,
        relevance: f32,
    ) -> Result<(), MemcpError> {
        // Satisfy FK constraint — ensure_session is idempotent.
        self.ensure_session(session_id).await?;

        sqlx::query(
            "INSERT INTO session_recalls (session_id, memory_id, recalled_at, relevance) \
             VALUES ($1, $2, NOW(), $3) \
             ON CONFLICT (session_id, memory_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(memory_id)
        .bind(relevance)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to insert session recall: {}", e)))?;
        Ok(())
    }

    /// Clear all recall records for a session and reset its last_active_at.
    ///
    /// Also clears accumulated session_tags so subsequent recalls start with a clean
    /// topic slate (Pitfall 4 from RESEARCH.md: stale session tags should not bias
    /// fresh session after reset).
    ///
    /// Resetting last_active_at prevents the session from being immediately
    /// re-expired by the cleanup worker right after a reset (Pitfall 3 from RESEARCH.md).
    pub async fn clear_session_recalls(&self, session_id: &str) -> Result<(), MemcpError> {
        sqlx::query("DELETE FROM session_recalls WHERE session_id = $1")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to clear session recalls: {}", e)))?;

        sqlx::query(
            "UPDATE sessions SET last_active_at = NOW(), session_tags = NULL WHERE session_id = $1",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to touch session after clear: {}", e)))?;
        Ok(())
    }

    /// Read accumulated session tags for a session.
    ///
    /// Returns an empty Vec when the session has no accumulated tags or does not exist.
    /// Deduplication happens on read (simpler than SQL dedup — see RESEARCH.md Pitfall 3).
    pub async fn get_session_tags(&self, session_id: &str) -> Result<Vec<String>, MemcpError> {
        let row = sqlx::query(
            "SELECT COALESCE(session_tags, '[]'::jsonb) as tags FROM sessions WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get session tags: {}", e)))?;

        match row {
            None => Ok(vec![]),
            Some(row) => {
                let value: serde_json::Value = row.get("tags");
                let tags = serde_json::from_value::<Vec<String>>(value).unwrap_or_default();
                // Dedup on read — accumulate_session_tags appends without deduplication.
                let mut seen = std::collections::HashSet::new();
                let deduped = tags
                    .into_iter()
                    .filter(|t| seen.insert(t.clone()))
                    .collect();
                Ok(deduped)
            }
        }
    }

    /// Append new tags to the session's accumulated topic set.
    ///
    /// Tags are appended without deduplication — dedup happens on read in get_session_tags.
    /// No-op when new_tags is empty.
    pub async fn accumulate_session_tags(
        &self,
        session_id: &str,
        new_tags: &[String],
    ) -> Result<(), MemcpError> {
        if new_tags.is_empty() {
            return Ok(());
        }
        let tags_json = serde_json::Value::Array(
            new_tags
                .iter()
                .map(|t| serde_json::Value::String(t.clone()))
                .collect(),
        );
        sqlx::query(
            "UPDATE sessions SET session_tags = COALESCE(session_tags, '[]'::jsonb) || $2::jsonb WHERE session_id = $1",
        )
        .bind(session_id)
        .bind(tags_json)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to accumulate session tags: {}", e)))?;
        Ok(())
    }

    /// Delete sessions that have been idle longer than idle_secs.
    ///
    /// CASCADE on session_recalls FK means deleting a session automatically
    /// removes all its recall records — no separate cleanup needed.
    /// Returns the number of sessions deleted.
    /// Called from the GC worker alongside memory pruning.
    pub async fn cleanup_expired_sessions(&self, idle_secs: u64) -> Result<u64, MemcpError> {
        let result = sqlx::query(
            "DELETE FROM sessions WHERE last_active_at < NOW() - ($1 || ' seconds')::INTERVAL",
        )
        .bind(idle_secs.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to clean up expired sessions: {}", e)))?;
        Ok(result.rows_affected())
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

        let mut tx = self.pool.begin().await.map_err(|e| {
            MemcpError::Storage(format!("Failed to begin dedup transaction: {}", e))
        })?;

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
            MemcpError::Storage(format!(
                "Failed to update existing memory in dedup merge: {}",
                e
            ))
        })?;

        // Soft-delete the incoming duplicate
        sqlx::query("UPDATE memories SET deleted_at = NOW() WHERE id = $1")
            .bind(new_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!(
                    "Failed to soft-delete duplicate in dedup merge: {}",
                    e
                ))
            })?;

        tx.commit().await.map_err(|e| {
            MemcpError::Storage(format!("Failed to commit dedup merge transaction: {}", e))
        })?;

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

    /// Delete all chunks belonging to a parent memory.
    ///
    /// Used for re-chunking when parent content changes.
    /// Returns the number of deleted chunk rows.
    pub async fn delete_chunks_by_parent(&self, parent_id: &str) -> Result<u64, MemcpError> {
        let result = sqlx::query("DELETE FROM memories WHERE parent_id = $1")
            .bind(parent_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!(
                    "Failed to delete chunks for parent {}: {}",
                    parent_id, e
                ))
            })?;
        Ok(result.rows_affected())
    }

    // ═══════════════════════════════════════════════════════════════════
    // Curation store methods
    // ═══════════════════════════════════════════════════════════════════

    /// Get the window_end of the last successful curation run.
    /// Returns None if no completed run exists (triggers full-corpus first run).
    pub async fn get_last_successful_curation_time(
        &self,
    ) -> Result<Option<DateTime<Utc>>, MemcpError> {
        let row = sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT window_end FROM curation_runs WHERE status = 'completed' \
             ORDER BY completed_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get last curation time: {}", e)))?;
        Ok(row)
    }

    /// Fetch candidate memories for curation with their salience data.
    /// Windowed: only memories created/modified since `since` (or all if None).
    /// Excludes soft-deleted, pending-embedding, and recently-created curated memories.
    pub async fn get_memories_for_curation(
        &self,
        since: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<(crate::store::Memory, SalienceRow)>, MemcpError> {
        let query = if since.is_some() {
            "SELECT m.id, m.content, m.type_hint, m.source, m.tags, m.created_at, m.updated_at, \
             m.last_accessed_at, m.access_count, m.embedding_status, \
             m.extracted_entities, m.extracted_facts, m.extraction_status, \
             m.is_consolidated_original, m.consolidated_into, \
             m.actor, m.actor_type, m.audience, \
             m.event_time, m.event_time_precision, m.project, \
             m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
             m.abstract_text, m.overview_text, m.abstraction_status, \
             COALESCE(s.stability, 1.0) as sal_stability, \
             COALESCE(s.difficulty, 5.0) as sal_difficulty, \
             COALESCE(s.reinforcement_count, 0) as sal_reinforcement_count, \
             s.last_reinforced_at as sal_last_reinforced_at \
             FROM memories m \
             LEFT JOIN memory_salience s ON m.id = s.memory_id \
             WHERE m.deleted_at IS NULL \
             AND m.embedding_status = 'complete' \
             AND NOT (m.type_hint = 'curated' AND m.created_at > NOW() - INTERVAL '1 hour') \
             AND (m.tags IS NULL OR NOT m.tags @> to_jsonb('curation:reviewed'::text)) \
             AND (m.updated_at > $1 OR m.created_at > $1) \
             ORDER BY m.created_at ASC \
             LIMIT $2"
        } else {
            "SELECT m.id, m.content, m.type_hint, m.source, m.tags, m.created_at, m.updated_at, \
             m.last_accessed_at, m.access_count, m.embedding_status, \
             m.extracted_entities, m.extracted_facts, m.extraction_status, \
             m.is_consolidated_original, m.consolidated_into, \
             m.actor, m.actor_type, m.audience, \
             m.event_time, m.event_time_precision, m.project, \
             m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
             m.abstract_text, m.overview_text, m.abstraction_status, \
             COALESCE(s.stability, 1.0) as sal_stability, \
             COALESCE(s.difficulty, 5.0) as sal_difficulty, \
             COALESCE(s.reinforcement_count, 0) as sal_reinforcement_count, \
             s.last_reinforced_at as sal_last_reinforced_at \
             FROM memories m \
             LEFT JOIN memory_salience s ON m.id = s.memory_id \
             WHERE m.deleted_at IS NULL \
             AND m.embedding_status = 'complete' \
             AND NOT (m.type_hint = 'curated' AND m.created_at > NOW() - INTERVAL '1 hour') \
             AND (m.tags IS NULL OR NOT m.tags @> to_jsonb('curation:reviewed'::text)) \
             ORDER BY m.created_at ASC \
             LIMIT $1"
        };

        let rows = if let Some(since_time) = since {
            sqlx::query(query)
                .bind(since_time)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
        } else {
            sqlx::query(query)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await
        }
        .map_err(|e| MemcpError::Storage(format!("Failed to get memories for curation: {}", e)))?;

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let memory = row_to_memory(row)?;
            let salience = SalienceRow {
                stability: row.try_get::<f64, _>("sal_stability").unwrap_or(1.0),
                difficulty: row.try_get::<f64, _>("sal_difficulty").unwrap_or(5.0),
                reinforcement_count: row
                    .try_get::<i32, _>("sal_reinforcement_count")
                    .unwrap_or(0),
                last_reinforced_at: row
                    .try_get::<Option<DateTime<Utc>>, _>("sal_last_reinforced_at")
                    .unwrap_or(None),
            };
            results.push((memory, salience));
        }
        Ok(results)
    }

    /// Create a new curation run record. Returns the run_id (UUID).
    pub async fn create_curation_run(
        &self,
        mode: &str,
        window_start: Option<DateTime<Utc>>,
        window_end: DateTime<Utc>,
    ) -> Result<String, MemcpError> {
        let id: (uuid::Uuid,) = sqlx::query_as(
            "INSERT INTO curation_runs (mode, window_start, window_end) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(mode)
        .bind(window_start)
        .bind(window_end)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to create curation run: {}", e)))?;
        Ok(id.0.to_string())
    }

    /// Mark a curation run as completed with final counts.
    pub async fn complete_curation_run(
        &self,
        run_id: &str,
        merged_count: i32,
        flagged_stale_count: i32,
        strengthened_count: i32,
        skipped_count: i32,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE curation_runs SET status = 'completed', completed_at = NOW(), \
             merged_count = $2, flagged_stale_count = $3, \
             strengthened_count = $4, skipped_count = $5 \
             WHERE id = $1::uuid",
        )
        .bind(run_id)
        .bind(merged_count)
        .bind(flagged_stale_count)
        .bind(strengthened_count)
        .bind(skipped_count)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to complete curation run: {}", e)))?;
        Ok(())
    }

    /// Mark a curation run as failed.
    pub async fn fail_curation_run(&self, run_id: &str, error: &str) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE curation_runs SET status = 'failed', completed_at = NOW(), \
             error_message = $2 WHERE id = $1::uuid",
        )
        .bind(run_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to mark curation run as failed: {}", e))
        })?;
        Ok(())
    }

    /// Record a single curation action for undo tracking.
    pub async fn record_curation_action(
        &self,
        run_id: &str,
        action_type: &str,
        target_memory_ids: &[String],
        merged_memory_id: Option<&str>,
        original_salience: Option<f64>,
        details: Option<serde_json::Value>,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "INSERT INTO curation_actions \
             (run_id, action_type, target_memory_ids, merged_memory_id, original_salience, details) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6)",
        )
        .bind(run_id)
        .bind(action_type)
        .bind(target_memory_ids)
        .bind(merged_memory_id)
        .bind(original_salience)
        .bind(details)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to record curation action: {}", e))
        })?;
        Ok(())
    }

    /// Get recent curation runs for the log command.
    pub async fn get_curation_runs(&self, limit: usize) -> Result<Vec<CurationRunRow>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, started_at, completed_at, status, mode, \
             window_start, window_end, \
             merged_count, flagged_stale_count, strengthened_count, skipped_count, \
             error_message \
             FROM curation_runs ORDER BY started_at DESC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get curation runs: {}", e)))?;

        rows.iter()
            .map(|row| {
                Ok(CurationRunRow {
                    id: row
                        .try_get::<uuid::Uuid, _>("id")
                        .map(|u| u.to_string())
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    started_at: row
                        .try_get("started_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    completed_at: row
                        .try_get("completed_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    status: row
                        .try_get("status")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    mode: row
                        .try_get("mode")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    window_start: row
                        .try_get("window_start")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    window_end: row
                        .try_get("window_end")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    merged_count: row
                        .try_get("merged_count")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    flagged_stale_count: row
                        .try_get("flagged_stale_count")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    strengthened_count: row
                        .try_get("strengthened_count")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    skipped_count: row
                        .try_get("skipped_count")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    error_message: row
                        .try_get("error_message")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                })
            })
            .collect()
    }

    /// Get all actions for a specific curation run (for undo or detailed log).
    pub async fn get_curation_actions(
        &self,
        run_id: &str,
    ) -> Result<Vec<CurationActionRow>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, run_id, action_type, target_memory_ids, \
             merged_memory_id, original_salience, details, created_at \
             FROM curation_actions WHERE run_id = $1::uuid ORDER BY created_at ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get curation actions: {}", e)))?;

        rows.iter()
            .map(|row| {
                Ok(CurationActionRow {
                    id: row
                        .try_get::<uuid::Uuid, _>("id")
                        .map(|u| u.to_string())
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    run_id: row
                        .try_get::<uuid::Uuid, _>("run_id")
                        .map(|u| u.to_string())
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    action_type: row
                        .try_get("action_type")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    target_memory_ids: row
                        .try_get("target_memory_ids")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    merged_memory_id: row
                        .try_get("merged_memory_id")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    original_salience: row
                        .try_get("original_salience")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    details: row
                        .try_get("details")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    created_at: row
                        .try_get("created_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                })
            })
            .collect()
    }

    /// Undo all actions from a curation run.
    /// Restores soft-deleted originals, hard-deletes merged memories,
    /// restores original salience values, marks run as 'undone'.
    pub async fn undo_curation_run(&self, run_id: &str) -> Result<usize, MemcpError> {
        let actions = self.get_curation_actions(run_id).await?;
        if actions.is_empty() {
            return Err(MemcpError::Storage(format!(
                "No actions found for curation run {}",
                run_id
            )));
        }

        let mut reversed = 0usize;

        for action in &actions {
            match action.action_type.as_str() {
                "merge" => {
                    // Restore soft-deleted originals
                    if !action.target_memory_ids.is_empty() {
                        sqlx::query(
                            "UPDATE memories SET deleted_at = NULL \
                             WHERE id = ANY($1) AND deleted_at IS NOT NULL",
                        )
                        .bind(&action.target_memory_ids)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| {
                            MemcpError::Storage(format!("Failed to restore originals: {}", e))
                        })?;
                    }
                    // Hard-delete the merged memory
                    if let Some(merged_id) = &action.merged_memory_id {
                        sqlx::query("DELETE FROM memories WHERE id = $1")
                            .bind(merged_id)
                            .execute(&self.pool)
                            .await
                            .map_err(|e| {
                                MemcpError::Storage(format!(
                                    "Failed to delete merged memory: {}",
                                    e
                                ))
                            })?;
                    }
                    reversed += 1;
                }
                "flag_stale" => {
                    // Restore original salience
                    if let Some(original) = action.original_salience {
                        for target_id in &action.target_memory_ids {
                            self.update_memory_stability(target_id, original).await?;
                        }
                    }
                    // Remove 'stale' tag from target memories
                    for target_id in &action.target_memory_ids {
                        sqlx::query(
                            "UPDATE memories SET tags = \
                             (SELECT COALESCE(jsonb_agg(elem), '[]'::jsonb) \
                              FROM jsonb_array_elements(COALESCE(tags, '[]'::jsonb)) elem \
                              WHERE elem::text != '\"stale\"') \
                             WHERE id = $1",
                        )
                        .bind(target_id)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| {
                            MemcpError::Storage(format!("Failed to remove stale tag: {}", e))
                        })?;
                    }
                    reversed += 1;
                }
                "strengthen" => {
                    // Restore original salience
                    if let Some(original) = action.original_salience {
                        for target_id in &action.target_memory_ids {
                            self.update_memory_stability(target_id, original).await?;
                        }
                    }
                    // Remove 'curated:strengthened' tag
                    for target_id in &action.target_memory_ids {
                        sqlx::query(
                            "UPDATE memories SET tags = \
                             (SELECT COALESCE(jsonb_agg(elem), '[]'::jsonb) \
                              FROM jsonb_array_elements(COALESCE(tags, '[]'::jsonb)) elem \
                              WHERE elem::text != '\"curated:strengthened\"') \
                             WHERE id = $1",
                        )
                        .bind(target_id)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| {
                            MemcpError::Storage(format!("Failed to remove strengthen tag: {}", e))
                        })?;
                    }
                    reversed += 1;
                }
                _ => {} // skip actions need no undo
            }
        }

        // Mark run as undone
        sqlx::query("UPDATE curation_runs SET status = 'undone' WHERE id = $1::uuid")
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to mark run as undone: {}", e)))?;

        Ok(reversed)
    }

    /// Add a tag to a memory's tag array.
    pub async fn add_memory_tag(&self, memory_id: &str, tag: &str) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE memories SET tags = \
             CASE WHEN tags IS NULL THEN jsonb_build_array($2::text) \
             WHEN NOT tags @> to_jsonb($2::text) THEN tags || to_jsonb($2::text) \
             ELSE tags END, \
             updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(memory_id)
        .bind(tag)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to add tag to memory: {}", e)))?;
        Ok(())
    }

    /// Get all chunks for a parent memory, ordered by chunk_index.
    pub async fn get_chunks_by_parent(
        &self,
        parent_id: &str,
    ) -> Result<Vec<crate::store::Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, \
             last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, \
             is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status, knowledge_tier, source_ids \
             FROM memories WHERE parent_id = $1 AND deleted_at IS NULL \
             ORDER BY chunk_index ASC",
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!(
                "Failed to get chunks for parent {}: {}",
                parent_id, e
            ))
        })?;

        rows.iter().map(row_to_memory).collect()
    }

    // -----------------------------------------------------------------------
    // Related context
    // -----------------------------------------------------------------------

    /// For each memory ID, count how many other live memories share at least one
    /// non-trivial tag, and return the shared tags for use as a search hint.
    ///
    /// Trivial tags (e.g. "auto-stored", "summarized") are excluded from the hint.
    /// Uses a batch query to avoid N individual round-trips.
    pub async fn get_related_context(
        &self,
        memory_ids: &[String],
    ) -> Result<std::collections::HashMap<String, RelatedContext>, MemcpError> {
        if memory_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        // Tags to skip when building the hint — too common to be useful for navigation.
        const SKIP_TAGS: &[&str] = &[
            "auto-stored",
            "summarized",
            "merged",
            "stale",
            "curated:strengthened",
        ];

        // Step A: Fetch tags for all requested memory IDs in one query.
        let rows =
            sqlx::query("SELECT id, tags FROM memories WHERE id = ANY($1) AND deleted_at IS NULL")
                .bind(memory_ids)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    MemcpError::Storage(format!("get_related_context tag fetch failed: {}", e))
                })?;

        let mut result: std::collections::HashMap<String, RelatedContext> =
            std::collections::HashMap::new();

        for row in &rows {
            let memory_id: String = row.get("id");
            let tags_val: Option<serde_json::Value> = row.get("tags");

            // Extract non-trivial tags from this memory.
            let interesting_tags: Vec<String> = tags_val
                .as_ref()
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str())
                        .filter(|t| !SKIP_TAGS.contains(t) && !t.starts_with("category:"))
                        .map(|t| t.to_string())
                        .collect()
                })
                .unwrap_or_default();

            if interesting_tags.is_empty() {
                result.insert(
                    memory_id,
                    RelatedContext {
                        related_count: 0,
                        shared_tags: vec![],
                    },
                );
                continue;
            }

            // Step B: Count other memories sharing at least one of these tags.
            let count_row = sqlx::query(
                "SELECT COUNT(DISTINCT m2.id) AS related_count \
                 FROM memories m2 \
                 WHERE m2.id != $1 \
                   AND m2.deleted_at IS NULL \
                   AND m2.tags IS NOT NULL \
                   AND m2.tags ?| $2::text[]",
            )
            .bind(&memory_id)
            .bind(&interesting_tags)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!(
                    "get_related_context count failed for {}: {}",
                    memory_id, e
                ))
            })?;

            let related_count: i64 = count_row.get("related_count");

            result.insert(
                memory_id,
                RelatedContext {
                    related_count,
                    shared_tags: interesting_tags,
                },
            );
        }

        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Temporal update
    // -----------------------------------------------------------------------

    /// Update event_time and event_time_precision for a memory.
    ///
    /// Used by the temporal LLM background worker to backfill memories
    /// that had no regex-detectable temporal reference at store time.
    pub async fn update_event_time(
        &self,
        id: &str,
        event_time: chrono::DateTime<chrono::Utc>,
        precision: &str,
    ) -> Result<(), crate::errors::MemcpError> {
        sqlx::query(
            "UPDATE memories SET event_time = $2, event_time_precision = $3, updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .bind(event_time)
        .bind(precision)
        .execute(&self.pool)
        .await
        .map_err(|e| crate::errors::MemcpError::Storage(format!("Failed to update event_time for {}: {}", id, e)))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Provenance: trust level update with audit trail
    // -----------------------------------------------------------------------

    /// Remove a tag from a memory's tag array.
    pub async fn remove_memory_tag(&self, memory_id: &str, tag: &str) -> Result<(), MemcpError> {
        sqlx::query("UPDATE memories SET tags = tags - $2::text, updated_at = NOW() WHERE id = $1")
            .bind(memory_id)
            .bind(tag)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to remove tag from memory: {}", e)))?;
        Ok(())
    }

    /// Un-quarantine a memory: remove "suspicious" tag and restore previous trust_level
    /// from the trust_history audit trail.
    pub async fn unquarantine_memory(&self, memory_id: &str) -> Result<(), MemcpError> {
        // Read metadata to find the quarantine entry in trust_history
        let metadata: serde_json::Value = sqlx::query_scalar(
            "SELECT metadata FROM memories WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(memory_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to read metadata: {}", e)))?
        .ok_or_else(|| MemcpError::NotFound {
            id: memory_id.to_string(),
        })?;

        // Find the most recent quarantine entry's "from" value
        let previous_trust = metadata
            .get("trust_history")
            .and_then(|h| h.as_array())
            .and_then(|arr| {
                // Find the last entry where reason contains "quarantined" or to=0.05
                arr.iter().rev().find(|entry| {
                    entry
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .is_some_and(|r| r.contains("quarantined"))
                })
            })
            .and_then(|entry| entry.get("from"))
            .and_then(|f| f.as_f64())
            .unwrap_or(0.5) as f32; // fallback to 0.5 if no history found

        // Restore trust level
        self.update_trust_level(memory_id, previous_trust, "unquarantined")
            .await?;

        // Remove suspicious tag
        self.remove_memory_tag(memory_id, "suspicious").await?;

        Ok(())
    }

    pub async fn update_trust_level(
        &self,
        id: &str,
        new_trust: f32,
        reason: &str,
    ) -> Result<(), MemcpError> {
        // Read current trust_level
        let current_trust: f32 = sqlx::query_scalar(
            "SELECT trust_level FROM memories WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to read trust_level: {}", e)))?
        .ok_or_else(|| MemcpError::NotFound { id: id.to_string() })?;

        // Build the history entry
        let now = Utc::now();
        let history_entry = serde_json::json!({
            "from": current_trust,
            "to": new_trust,
            "reason": reason,
            "at": now.to_rfc3339(),
        });

        // Atomic update: set trust_level + append to metadata.trust_history
        sqlx::query(
            "UPDATE memories SET \
             trust_level = $2, \
             metadata = jsonb_set(\
                 metadata, \
                 '{trust_history}', \
                 COALESCE(metadata->'trust_history', '[]'::jsonb) || $3::jsonb \
             ), \
             updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(id)
        .bind(new_trust)
        .bind(history_entry)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update trust_level: {}", e)))?;

        Ok(())
    }
}
