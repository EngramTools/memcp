#![allow(clippy::unwrap_used)]
//! MemoryStore trait implementation for PostgresMemoryStore (core CRUD operations).

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::{row_to_memory, PostgresMemoryStore};
use crate::errors::MemcpError;
use crate::store::{CreateMemory, ListFilter, ListResult, Memory, MemoryStore, UpdateMemory};

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

/// Compute FNV-1a 64-bit hash of content string.
///
/// FNV-1a is deterministic, cross-process stable, and requires no external dependencies.
/// Used for content-hash dedup: identical content produces the same hex string.
fn content_hash(content: &str) -> String {
    let mut hash: u64 = 14695981039346656037;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{:016x}", hash)
}

#[async_trait]
impl MemoryStore for PostgresMemoryStore {
    /// Store a new memory with idempotency guarantees.
    ///
    /// Dedup priority (highest first):
    ///   1. Caller-provided idempotency_key: if key exists and is not expired, return original.
    ///   2. Content-hash dedup: if identical content was stored within dedup_window_secs, return existing.
    ///   3. Otherwise: insert new memory, write content_hash, register idempotency key if provided.
    ///
    /// Per CONTEXT.md locked decisions:
    ///   - Silent return: same response shape as new store — caller cannot distinguish dedup hit.
    ///   - No metadata updates: existing memory stays exactly as originally stored (true no-op).
    async fn store(&self, input: CreateMemory) -> Result<Memory, MemcpError> {
        // --- Step 1: Idempotency key lookup (highest priority) ---
        if let Some(ref key) = input.idempotency_key {
            let existing_row = sqlx::query(
                "SELECT m.id, m.content, m.type_hint, m.source, m.tags, m.created_at, m.updated_at, \
                 m.last_accessed_at, m.access_count, m.embedding_status, \
                 m.extracted_entities, m.extracted_facts, m.extraction_status, \
                 m.is_consolidated_original, m.consolidated_into, m.actor, m.actor_type, m.audience, \
                 m.parent_id, m.chunk_index, m.total_chunks, \
                 m.event_time, m.event_time_precision, m.project, \
                 m.trust_level, m.session_id, m.agent_role, m.write_path, m.metadata, \
                 m.abstract_text, m.overview_text, m.abstraction_status \
                 FROM idempotency_keys ik \
                 JOIN memories m ON m.id = ik.memory_id \
                 WHERE ik.key = $1 AND ik.expires_at > NOW() AND m.deleted_at IS NULL",
            )
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to query idempotency key: {}", e)))?;

            if let Some(row) = existing_row {
                tracing::info!(idempotency_key = %key, "store: idempotency key hit, returning existing memory");
                return row_to_memory(&row);
            }
        }

        // --- Step 2: Content-hash dedup ---
        let hash = content_hash(&input.content);
        if self.idempotency_config.dedup_window_secs > 0 {
            let window = self.idempotency_config.dedup_window_secs as i64;
            let existing_row = sqlx::query(
                "SELECT id, content, type_hint, source, tags, created_at, updated_at, \
                 last_accessed_at, access_count, embedding_status, \
                 extracted_entities, extracted_facts, extraction_status, \
                 is_consolidated_original, consolidated_into, actor, actor_type, audience, \
                 parent_id, chunk_index, total_chunks, \
                 event_time, event_time_precision, project, \
                 trust_level, session_id, agent_role, write_path, metadata, \
                 abstract_text, overview_text, abstraction_status \
                 FROM memories \
                 WHERE content_hash = $1 AND deleted_at IS NULL \
                   AND created_at > NOW() - ($2 || ' seconds')::interval \
                 ORDER BY created_at DESC \
                 LIMIT 1",
            )
            .bind(&hash)
            .bind(window.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to query content hash dedup: {}", e))
            })?;

            if let Some(row) = existing_row {
                let existing_id: String = row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                tracing::info!(content_hash = %hash, existing_id = %existing_id, "store: content-hash dedup hit, returning existing memory");
                return row_to_memory(&row);
            }
        }

        // --- Step 3: Insert new memory ---
        let id = Uuid::new_v4().to_string();
        let now = input.created_at.unwrap_or_else(Utc::now);

        // Convert tags Vec<String> to serde_json::Value for JSONB binding
        let tags_json: Option<serde_json::Value> =
            input.tags.as_ref().map(|t| serde_json::json!(t));

        // Resolve trust level: explicit value takes precedence, else infer from source/actor_type
        let resolved_trust = input
            .trust_level
            .unwrap_or_else(|| crate::store::infer_trust_level(&input.source, &input.actor_type));
        let empty_metadata = serde_json::json!({});

        // Determine abstraction_status at store time: skip short content (< 200 chars)
        let abstraction_status = if input.content.len() < 200 {
            "skipped"
        } else {
            "pending"
        };

        sqlx::query(
            "INSERT INTO memories (id, content, type_hint, source, tags, created_at, updated_at, access_count, embedding_status, actor, actor_type, audience, content_hash, parent_id, chunk_index, total_chunks, event_time, event_time_precision, project, trust_level, session_id, agent_role, write_path, metadata, abstraction_status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'pending', $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)",
        )
        .bind(&id)
        .bind(&input.content)
        .bind(&input.type_hint)
        .bind(&input.source)
        .bind(&tags_json)     // JSONB — bind serde_json::Value directly
        .bind(now)           // TIMESTAMPTZ — bind DateTime<Utc> directly
        .bind(now)
        .bind(&input.actor)
        .bind(&input.actor_type)
        .bind(&input.audience)
        .bind(&hash)          // content_hash for dedup
        .bind(&input.parent_id)
        .bind(input.chunk_index)
        .bind(input.total_chunks)
        .bind(input.event_time)
        .bind(&input.event_time_precision)
        .bind(&input.project)
        .bind(resolved_trust)
        .bind(&input.session_id)
        .bind(&input.agent_role)
        .bind(&input.write_path)
        .bind(&empty_metadata)
        .bind(abstraction_status)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to insert memory: {}", e)))?;

        // --- Step 4: Register idempotency key if provided ---
        if let Some(ref key) = input.idempotency_key {
            let ttl = self.idempotency_config.key_ttl_secs as i64;
            if let Err(e) = sqlx::query(
                "INSERT INTO idempotency_keys (key, memory_id, expires_at) \
                 VALUES ($1, $2, NOW() + ($3 || ' seconds')::interval) \
                 ON CONFLICT (key) DO NOTHING",
            )
            .bind(key)
            .bind(&id)
            .bind(ttl.to_string())
            .execute(&self.pool)
            .await
            {
                // Log but don't fail — the memory was already inserted
                tracing::warn!(
                    idempotency_key = %key,
                    error = %e,
                    "store: failed to register idempotency key (memory was still stored)"
                );
            }
        }

        // --- Step 5: Apply type-specific initial FSRS stability (if retention config set) ---
        if let Some(ref retention) = self.retention_config {
            let type_hint = input.type_hint.as_str();
            let stability = retention.stability_for_type(type_hint);
            // Only write if different from default (2.5) to avoid unnecessary DB writes
            if (stability - 2.5).abs() > 0.01 {
                if let Err(e) = self.update_memory_stability(&id, stability).await {
                    // Log but don't fail — memory is stored, salience row may be missing
                    tracing::warn!(
                        memory_id = %id,
                        type_hint = %type_hint,
                        stability = stability,
                        error = %e,
                        "store: failed to set type-specific initial stability (memory was still stored)"
                    );
                } else {
                    tracing::debug!(
                        memory_id = %id,
                        type_hint = %type_hint,
                        stability = stability,
                        "store: applied type-specific initial FSRS stability"
                    );
                }
            }
        }

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
            parent_id: input.parent_id,
            chunk_index: input.chunk_index,
            total_chunks: input.total_chunks,
            event_time: input.event_time,
            event_time_precision: input.event_time_precision,
            project: input.project,
            trust_level: resolved_trust,
            session_id: input.session_id,
            agent_role: input.agent_role,
            write_path: input.write_path,
            metadata: serde_json::json!({}),
            abstract_text: None,
            overview_text: None,
            // abstraction_status was computed above and used in the INSERT
            abstraction_status: abstraction_status.to_string(),
        })
    }

    async fn get(&self, id: &str) -> Result<Memory, MemcpError> {
        let row = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
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
        if input.trust_level.is_some() {
            sets.push(format!("trust_level = ${}", param_idx));
            param_idx += 1;
        }

        let sql = format!(
            "UPDATE memories SET {} WHERE id = ${}",
            sets.join(", "),
            param_idx
        );

        let mut q = sqlx::query(&sql).bind(now); // $1 = updated_at
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
        if let Some(trust) = input.trust_level {
            q = q.bind(trust);
        }
        q = q.bind(id); // final $N = id

        q.execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to update memory: {}", e)))?;

        // Re-fetch and return the updated record
        let updated_row = sqlx::query(
            "SELECT id, content, type_hint, source, tags, created_at, updated_at, last_accessed_at, access_count, embedding_status, \
             extracted_entities, extracted_facts, extraction_status, is_consolidated_original, consolidated_into, \
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
             FROM memories WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(e.to_string()))?;

        row_to_memory(&updated_row)
    }

    /// Delete a memory by ID. Idempotent: returns Ok(()) even if the memory does not exist.
    ///
    /// Per IDP-03: callers (MCP sandboxes, CLI) can safely retry delete without errors.
    /// If the memory already doesn't exist (or was already deleted), this is a silent no-op.
    async fn delete(&self, id: &str) -> Result<(), MemcpError> {
        sqlx::query("DELETE FROM memories WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;

        // Idempotent: rows_affected == 0 is fine — memory already didn't exist
        Ok(())
    }

    async fn list(&self, filter: ListFilter) -> Result<ListResult, MemcpError> {
        let limit = filter.limit.clamp(1, 100);

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
                param_idx,
                param_idx + 1,
                param_idx + 2
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
        if filter.project.is_some() {
            // Project-scoped: return memories from this project OR global (NULL project).
            conditions.push(format!("(project = ${} OR project IS NULL)", param_idx));
            param_idx += 1;
        }
        if filter.session_id.is_some() {
            conditions.push(format!("session_id = ${}", param_idx));
            param_idx += 1;
        }
        if filter.agent_role.is_some() {
            conditions.push(format!("agent_role = ${}", param_idx));
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
             actor, actor_type, audience, parent_id, chunk_index, total_chunks, \
             event_time, event_time_precision, project, \
             trust_level, session_id, agent_role, write_path, metadata, \
             abstract_text, overview_text, abstraction_status \
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
        if let Some(ref project) = filter.project {
            q = q.bind(project);
        }
        if let Some(ref session_id) = filter.session_id {
            q = q.bind(session_id);
        }
        if let Some(ref agent_role) = filter.agent_role {
            q = q.bind(agent_role);
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

        let count: i64 = row
            .try_get("count")
            .map_err(|e| MemcpError::Storage(e.to_string()))?;
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
        .bind(now) // TIMESTAMPTZ — bind DateTime<Utc> directly
        .bind(id)
        .execute(&self.pool)
        .await;

        Ok(())
    }
}

impl PostgresMemoryStore {
    /// Resolve a short ID prefix to a full memory ID.
    ///
    /// Agents using TOON persona blocks see truncated 8-char memory IDs.
    /// This method resolves a prefix to the full UUID before passing to store operations.
    ///
    /// Behavior:
    /// - 0 matches → Err("memory not found for prefix '{prefix}'")
    /// - 1 match → Ok(full_id)
    /// - 2+ matches → Err("ambiguous prefix '{prefix}' — matches multiple memories, use full ID")
    ///
    /// LIMIT 2 is sufficient: we only need to detect uniqueness vs. ambiguity.
    pub async fn resolve_id_prefix(&self, prefix: &str) -> Result<String, MemcpError> {
        let rows = sqlx::query(
            "SELECT id FROM memories WHERE id LIKE $1 || '%' AND deleted_at IS NULL LIMIT 2",
        )
        .bind(prefix)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to resolve ID prefix: {}", e)))?;

        match rows.len() {
            0 => Err(MemcpError::NotFound {
                id: format!("memory not found for prefix '{}'", prefix),
            }),
            1 => {
                let full_id: String = rows[0]
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?;
                Ok(full_id)
            }
            _ => Err(MemcpError::Validation {
                message: format!(
                    "ambiguous prefix '{}' — matches multiple memories, use full ID",
                    prefix
                ),
                field: Some("id".to_string()),
            }),
        }
    }
}
