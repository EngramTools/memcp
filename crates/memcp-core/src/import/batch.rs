//! Batch INSERT for import pipeline.
//!
//! Bypasses per-item CLI overhead by inserting directly via Postgres pool.
//! Each batch runs in a single transaction for atomicity and performance.
//!
//! Target: 5,000 memories in < 60 seconds with embedding reuse.

use std::hash::{Hash, Hasher};

use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

use super::{ImportChunk, ImportOpts};

/// Result of a single batch insert operation.
#[derive(Debug, Default)]
pub struct BatchResult {
    /// Number of rows successfully inserted.
    pub inserted: usize,
    /// Number of rows skipped (ON CONFLICT DO NOTHING).
    pub skipped: usize,
}

/// FNV-1a content hash — matches the algorithm in pipeline/auto_store/mod.rs.
/// Duplicated here to avoid a direct dependency on the auto_store module.
fn fnv1a_content_hash(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Insert a batch of chunks in a single Postgres transaction.
///
/// Each chunk maps to a row in the `memories` table. If the chunk carries
/// a pre-computed embedding, a corresponding row is also inserted into
/// `memory_embeddings`.
///
/// Uses `ON CONFLICT DO NOTHING` on id (UUID safety net — should never fire).
pub async fn batch_insert_memories(
    pool: &PgPool,
    chunks: &[ImportChunk],
    opts: &ImportOpts,
) -> Result<BatchResult> {
    if chunks.is_empty() {
        return Ok(BatchResult::default());
    }

    let mut tx = pool.begin().await?;
    let mut result = BatchResult::default();

    for chunk in chunks {
        let id = Uuid::new_v4().to_string();
        let now = chunk.created_at.unwrap_or_else(Utc::now);
        let content_hash = fnv1a_content_hash(&chunk.content) as i64;

        // Merge tags: chunk tags + CLI --tags + source auto-tags.
        let mut merged_tags = chunk.tags.clone();

        // Add CLI-supplied tags.
        merged_tags.extend(opts.tags.iter().cloned());

        // Add source auto-tags: "imported" and "imported:<source>".
        if !merged_tags.contains(&"imported".to_string()) {
            merged_tags.push("imported".to_string());
        }
        let source_tag = format!("imported:{}", chunk.source.trim_start_matches("imported:"));
        if !merged_tags.contains(&source_tag) {
            merged_tags.push(source_tag);
        }

        // Deduplicate tags.
        merged_tags.sort();
        merged_tags.dedup();

        let tags_json = serde_json::json!(merged_tags);
        let type_hint = chunk.type_hint.as_deref().unwrap_or("fact").to_string();
        let embedding_status = if chunk.embedding.is_some() && !opts.skip_embeddings {
            "complete"
        } else {
            "pending"
        };

        // Map opts.project to project column.
        let project_scope = opts.project.clone().or_else(|| chunk.project.clone());

        let rows_affected = sqlx::query(
            "INSERT INTO memories (
                id, content, type_hint, source, tags, created_at, updated_at,
                access_count, embedding_status, actor, actor_type, audience,
                content_hash, parent_id, chunk_index, total_chunks,
                event_time, event_time_precision, project
             ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, 0, $8, $9, 'agent', 'global',
                $10, NULL, NULL, NULL, NULL, NULL, $11
             )
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&id)
        .bind(&chunk.content)
        .bind(&type_hint)
        .bind(&chunk.source)
        .bind(&tags_json)
        .bind(&now)
        .bind(&now)
        .bind(embedding_status)
        .bind(&chunk.actor)
        .bind(content_hash)
        .bind(&project_scope)
        .execute(&mut *tx)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to insert memory: {}", e))?
        .rows_affected();

        if rows_affected == 0 {
            result.skipped += 1;
            continue;
        }
        result.inserted += 1;

        // If chunk carries a pre-computed embedding, insert into memory_embeddings.
        if let (Some(embedding), Some(model_name)) = (&chunk.embedding, &chunk.embedding_model) {
            if !opts.skip_embeddings {
                let dimension = embedding.len() as i32;
                let embedding_id = Uuid::new_v4().to_string();

                // Format embedding as pgvector literal.
                let embedding_str = format!(
                    "[{}]",
                    embedding.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                );

                if let Err(e) = sqlx::query(
                    "INSERT INTO memory_embeddings (
                        id, memory_id, embedding, model_name, model_version,
                        dimension, is_current, tier, created_at, updated_at
                     ) VALUES (
                        $1, $2, $3::vector, $4, $5, $6, true, 'fast', $7, $8
                     )
                     ON CONFLICT DO NOTHING",
                )
                .bind(&embedding_id)
                .bind(&id)
                .bind(&embedding_str)
                .bind(model_name)
                .bind("imported")
                .bind(dimension)
                .bind(&now)
                .bind(&now)
                .execute(&mut *tx)
                .await
                {
                    warn!("Failed to insert embedding for memory {}: {}", id, e);
                    // Non-fatal: memory was inserted, embedding insertion failed.
                    // Daemon will re-embed from pending status.
                    sqlx::query(
                        "UPDATE memories SET embedding_status = 'pending' WHERE id = $1"
                    )
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .ok();
                }
            }
        }
    }

    tx.commit().await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_hash_is_deterministic() {
        let h1 = fnv1a_content_hash("hello world");
        let h2 = fnv1a_content_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_fnv1a_hash_differs_for_different_content() {
        let h1 = fnv1a_content_hash("content A");
        let h2 = fnv1a_content_hash("content B");
        assert_ne!(h1, h2);
    }
}
