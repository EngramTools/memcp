#![allow(clippy::unwrap_used)]
//! Salience scoring, weight management, and recall for PostgresMemoryStore.

use chrono::{DateTime, Utc};
use sqlx::Row;
use std::collections::HashMap;

use super::{PostgresMemoryStore, QuerylessCandidate, RecallCandidate, SalienceRow};
use crate::errors::MemcpError;

impl PostgresMemoryStore {
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
        .bind(now)
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
        let days_elapsed = current
            .last_reinforced_at
            .map(|dt| {
                let duration = Utc::now().signed_duration_since(dt);
                (duration.num_seconds() as f64 / 86_400.0).max(0.0)
            })
            .unwrap_or(365.0);

        // 3. Compute current retrievability (how fresh is the memory right now?)
        let retrievability =
            crate::search::salience::fsrs_retrievability(current.stability, days_elapsed);

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
        .bind(now)
        .bind(now)
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

    /// Apply a lightweight salience bump to a recalled memory.
    ///
    /// Variant of touch_salience with configurable multiplier and ceiling:
    ///   new_stability = min(old_stability * (1.0 + bump_multiplier), stability_ceiling)
    ///
    /// Does NOT update last_reinforced_at or reinforcement_count — this is a
    /// passive implicit signal, not explicit user reinforcement (matching touch_salience semantics).
    pub async fn recall_bump_salience(
        &self,
        memory_id: &str,
        bump_multiplier: f64,
        stability_ceiling: f64,
    ) -> Result<(), MemcpError> {
        let sql = "INSERT INTO memory_salience (memory_id, stability, updated_at) \
            VALUES ($1, LEAST(1.0 * (1.0 + $2), $3), NOW()) \
            ON CONFLICT (memory_id) \
            DO UPDATE SET stability = LEAST(memory_salience.stability * (1.0 + $2), $3), \
            updated_at = NOW()";
        sqlx::query(sql)
            .bind(memory_id)
            .bind(bump_multiplier)
            .bind(stability_ceiling)
            .execute(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(e.to_string()))?;
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
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to apply feedback: {}", e)))?;

        Ok(())
    }

    /// Directly set the stability value for a memory (used for stale demotion).
    pub async fn update_memory_stability(
        &self,
        memory_id: &str,
        stability: f64,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "INSERT INTO memory_salience (memory_id, stability, difficulty, \
             reinforcement_count, last_reinforced_at, created_at, updated_at) \
             VALUES ($1, $2, 5.0, 0, NULL, NOW(), NOW()) \
             ON CONFLICT (memory_id) DO UPDATE SET \
             stability = EXCLUDED.stability, updated_at = NOW()",
        )
        .bind(memory_id)
        .bind(stability)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to update memory stability: {}", e)))?;
        Ok(())
    }

    /// Query recall candidates using a tiered strategy, excluding already-recalled memories.
    ///
    /// Two tiers:
    /// - Extraction enabled: query against extracted_facts (one fact per memory via DISTINCT ON).
    /// - Extraction disabled: filter to type_hint IN ('fact', 'summary') or source = 'assistant'.
    ///
    /// Session dedup: memories already in session_recalls for this session are excluded via LEFT JOIN.
    ///
    /// Returns Vec of (memory_id, content, relevance, tags) tuples sorted by relevance DESC, stability DESC.
    /// Tags are included so the recall engine can apply tag-affinity boost without N+1 queries.
    pub async fn recall_candidates(
        &self,
        query_embedding: &[f32],
        session_id: &str,
        min_relevance: f64,
        max_memories: usize,
        extraction_enabled: bool,
        project: Option<&str>,
    ) -> Result<Vec<RecallCandidate>, MemcpError> {
        // Serialize embedding to pgvector literal format: '[0.1,0.2,...]'
        let emb_str = format!(
            "[{}]",
            query_embedding
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let limit = max_memories as i64;

        let project_clause = if project.is_some() {
            " AND (m.project = $5 OR m.project IS NULL)"
        } else {
            ""
        };

        if extraction_enabled {
            let sql = format!(
                "
                SELECT memory_id, content, relevance, tags, trust_level FROM (
                    SELECT DISTINCT ON (m.id)
                        m.id AS memory_id,
                        ef.fact AS content,
                        (1.0 - (me.embedding <=> $1::vector)) AS relevance,
                        COALESCE(ms.stability, 1.0) AS stability,
                        m.tags AS tags,
                        m.trust_level
                    FROM memories m
                    JOIN memory_embeddings me ON me.memory_id = m.id AND me.is_current = true
                    LEFT JOIN memory_salience ms ON ms.memory_id = m.id
                    LEFT JOIN session_recalls sr ON sr.session_id = $2 AND sr.memory_id = m.id
                    CROSS JOIN LATERAL jsonb_array_elements_text(m.extracted_facts) AS ef(fact)
                    WHERE m.deleted_at IS NULL
                      AND m.embedding_status = 'complete'
                      AND sr.memory_id IS NULL
                      AND m.extracted_facts IS NOT NULL
                      AND jsonb_array_length(m.extracted_facts) > 0
                      AND (1.0 - (me.embedding <=> $1::vector)) >= $3{project_clause}
                    ORDER BY m.id, (1.0 - (me.embedding <=> $1::vector)) DESC
                ) sub
                ORDER BY relevance DESC, stability DESC
                LIMIT $4
            "
            );
            let mut q = sqlx::query(&sql)
                .bind(&emb_str)
                .bind(session_id)
                .bind(min_relevance)
                .bind(limit);
            if let Some(ws) = project {
                q = q.bind(ws);
            }
            let rows = q.fetch_all(&self.pool).await.map_err(|e| {
                MemcpError::Storage(format!("recall_candidates (extraction) failed: {}", e))
            })?;

            let results = rows
                .iter()
                .map(|row| {
                    let memory_id: String = row.get("memory_id");
                    let content: String = row.get("content");
                    let relevance: f32 = row
                        .try_get::<f64, _>("relevance")
                        .map(|v| v as f32)
                        .or_else(|_| row.try_get::<f32, _>("relevance"))
                        .unwrap_or(0.0);
                    let tags: Option<serde_json::Value> = row.try_get("tags").ok().flatten();
                    let trust_level: f32 = row.try_get::<f32, _>("trust_level").unwrap_or(0.5);
                    RecallCandidate { memory_id, content, relevance, tags, trust_level }
                })
                .collect();
            Ok(results)
        } else {
            let sql = format!(
                "
                SELECT
                    m.id AS memory_id,
                    m.content,
                    (1.0 - (me.embedding <=> $1::vector)) AS relevance,
                    COALESCE(ms.stability, 1.0) AS stability,
                    m.tags AS tags,
                    m.trust_level
                FROM memories m
                JOIN memory_embeddings me ON me.memory_id = m.id AND me.is_current = true
                LEFT JOIN memory_salience ms ON ms.memory_id = m.id
                LEFT JOIN session_recalls sr ON sr.session_id = $2 AND sr.memory_id = m.id
                WHERE m.deleted_at IS NULL
                  AND m.embedding_status = 'complete'
                  AND sr.memory_id IS NULL
                  AND (m.type_hint IN ('fact', 'summary') OR m.source = 'assistant')
                  AND (1.0 - (me.embedding <=> $1::vector)) >= $3{project_clause}
                ORDER BY relevance DESC, stability DESC
                LIMIT $4
            "
            );
            let mut q = sqlx::query(&sql)
                .bind(&emb_str)
                .bind(session_id)
                .bind(min_relevance)
                .bind(limit);
            if let Some(ws) = project {
                q = q.bind(ws);
            }
            let rows = q.fetch_all(&self.pool).await.map_err(|e| {
                MemcpError::Storage(format!("recall_candidates (no-extraction) failed: {}", e))
            })?;

            let results = rows
                .iter()
                .map(|row| {
                    let memory_id: String = row.get("memory_id");
                    let content: String = row.get("content");
                    let relevance: f32 = row
                        .try_get::<f64, _>("relevance")
                        .map(|v| v as f32)
                        .or_else(|_| row.try_get::<f32, _>("relevance"))
                        .unwrap_or(0.0);
                    let tags: Option<serde_json::Value> = row.try_get("tags").ok().flatten();
                    let trust_level: f32 = row.try_get::<f32, _>("trust_level").unwrap_or(0.5);
                    RecallCandidate { memory_id, content, relevance, tags, trust_level }
                })
                .collect();
            Ok(results)
        }
    }

    /// Multi-tier recall candidates: embed the query with all tiers, search each tier,
    /// merge results by best relevance, and deduplicate by session.
    pub async fn recall_candidates_multi_tier(
        &self,
        tier_embeddings: &HashMap<String, Vec<f32>>,
        session_id: &str,
        min_relevance: f64,
        max_memories: usize,
        extraction_enabled: bool,
        project: Option<&str>,
    ) -> Result<Vec<RecallCandidate>, MemcpError> {
        if tier_embeddings.is_empty() {
            return Ok(vec![]);
        }

        if tier_embeddings.len() == 1 {
            let embedding = tier_embeddings.values().next().unwrap();
            return self
                .recall_candidates(
                    embedding,
                    session_id,
                    min_relevance,
                    max_memories,
                    extraction_enabled,
                    project,
                )
                .await;
        }

        // Multi-tier: query each tier separately and merge by best relevance.
        let mut merged: HashMap<String, (String, f32, Option<serde_json::Value>, f32)> =
            HashMap::new();

        for embedding in tier_embeddings.values() {
            let tier_results = self
                .recall_candidates(
                    embedding,
                    session_id,
                    min_relevance,
                    max_memories,
                    extraction_enabled,
                    project,
                )
                .await?;

            for candidate in tier_results {
                merged
                    .entry(candidate.memory_id)
                    .and_modify(|(_, best_rel, _, _)| {
                        if candidate.relevance > *best_rel {
                            *best_rel = candidate.relevance;
                        }
                    })
                    .or_insert((
                        candidate.content,
                        candidate.relevance,
                        candidate.tags,
                        candidate.trust_level,
                    ));
            }
        }

        // Sort by relevance descending and cap at max_memories
        let mut results: Vec<RecallCandidate> = merged
            .into_iter()
            .map(|(id, (content, rel, tags, trust))| RecallCandidate {
                memory_id: id,
                content,
                relevance: rel,
                tags,
                trust_level: trust,
            })
            .collect();
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_memories);

        Ok(results)
    }

    /// Query-less recall candidates: ranked by salience (stability DESC, updated_at DESC)
    /// without any vector search.
    pub async fn recall_candidates_queryless(
        &self,
        session_id: &str,
        overfetch_limit: usize,
        project: Option<&str>,
    ) -> Result<Vec<QuerylessCandidate>, MemcpError> {
        let limit = overfetch_limit as i64;
        let project_clause = if project.is_some() {
            " AND (m.project = $3 OR m.project IS NULL)"
        } else {
            ""
        };

        let sql = format!(
            "
            SELECT
                m.id AS memory_id,
                m.content,
                m.updated_at,
                m.access_count,
                COALESCE(ms.stability, 1.0) AS stability,
                ms.last_reinforced_at,
                m.tags,
                m.trust_level
            FROM memories m
            LEFT JOIN memory_salience ms ON ms.memory_id = m.id
            LEFT JOIN session_recalls sr ON sr.session_id = $1 AND sr.memory_id = m.id
            WHERE m.deleted_at IS NULL
              AND m.embedding_status = 'complete'
              AND sr.memory_id IS NULL
              AND (m.tags IS NULL OR NOT (m.tags @> '[\"project-summary\"]'::jsonb))
              {project_clause}
            ORDER BY COALESCE(ms.stability, 1.0) DESC, m.updated_at DESC
            LIMIT $2
        "
        );

        let mut q = sqlx::query(&sql).bind(session_id).bind(limit);
        if let Some(ws) = project {
            q = q.bind(ws);
        }

        let rows = q.fetch_all(&self.pool).await.map_err(|e| {
            MemcpError::Storage(format!("recall_candidates_queryless failed: {}", e))
        })?;

        let results = rows
            .iter()
            .map(|row| {
                let memory_id: String = row.get("memory_id");
                let content: String = row.get("content");
                let updated_at: chrono::DateTime<Utc> = row.get("updated_at");
                let access_count: i64 = row.get("access_count");
                let stability: f64 = row.try_get::<f64, _>("stability").unwrap_or(1.0);
                let last_reinforced_at: Option<chrono::DateTime<Utc>> =
                    row.get("last_reinforced_at");
                let tags: Option<serde_json::Value> = row.get("tags");
                let trust_level: f32 = row.try_get::<f32, _>("trust_level").unwrap_or(0.5);
                QuerylessCandidate {
                    memory_id,
                    content,
                    updated_at,
                    access_count,
                    stability,
                    last_reinforced_at,
                    tags,
                    trust_level,
                }
            })
            .collect();

        Ok(results)
    }

    /// Fetch the most recent memory tagged `project-summary` for the given project scope.
    ///
    /// Returns `(memory_id, content)` or `None` if no project-summary memory exists.
    pub async fn fetch_project_summary(
        &self,
        project: Option<&str>,
    ) -> Result<Option<(String, String)>, MemcpError> {
        let project_clause = if project.is_some() {
            " AND (m.project = $1 OR m.project IS NULL)"
        } else {
            ""
        };

        let sql = format!(
            "
            SELECT m.id, m.content
            FROM memories m
            WHERE m.deleted_at IS NULL
              AND m.tags @> '[\"project-summary\"]'::jsonb
              {project_clause}
            ORDER BY m.updated_at DESC
            LIMIT 1
        "
        );

        let mut q = sqlx::query(&sql);
        if let Some(ws) = project {
            q = q.bind(ws);
        }

        let row = q
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("fetch_project_summary failed: {}", e)))?;

        Ok(row.map(|r| {
            let id: String = r.get("id");
            let content: String = r.get("content");
            (id, content)
        }))
    }
}
