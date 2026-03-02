//! Recall engine — automatic context injection with session-scoped dedup.
//!
//! `RecallEngine::recall()` implements the tiered recall strategy:
//! - Extraction enabled: query against extracted_facts (compact fact content).
//! - Extraction disabled: filter to type_hint IN (fact, summary) memories.
//!
//! Session dedup: memories recalled within a session are not re-injected.
//! Implicit salience bump: recalled memories get a lightweight stability boost
//! (x1.15 by default, lighter than explicit reinforce at x1.5).
//!
//! `RecallEngine::recall_queryless()` supports cold-start context injection:
//! no query embedding required — memories are ranked purely by salience
//! (stability + recency + access + reinforcement). Optionally pins a
//! `project-summary` tagged memory as a separate `summary` field.

use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::config::{RecallConfig, SalienceConfig};
use crate::errors::MemcpError;
use crate::search::salience::{SalienceInput, SalienceScorer, ScoredHit};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::Memory;

/// A single memory returned by the recall engine.
///
/// Contains only the fields needed for context injection: id, content, relevance.
#[derive(Debug, Clone, Serialize)]
pub struct RecalledMemory {
    pub memory_id: String,
    pub content: String,
    pub relevance: f32,
}

/// The result of a recall operation.
#[derive(Debug, Clone, Serialize)]
pub struct RecallResult {
    pub session_id: String,
    pub count: usize,
    pub memories: Vec<RecalledMemory>,
    /// Pinned project summary — populated when `first=true` and a memory tagged
    /// `project-summary` exists. Not counted in `count`. Serde serializes None as null.
    pub summary: Option<RecalledMemory>,
}

/// Engine for automatic context injection via tiered recall strategy.
///
/// Created with a store reference, recall config, and extraction flag.
/// The `recall()` method handles the full pipeline: session management,
/// candidate query, dedup recording, and async salience bump.
pub struct RecallEngine {
    store: Arc<PostgresMemoryStore>,
    config: RecallConfig,
    extraction_enabled: bool,
}

impl RecallEngine {
    /// Create a new RecallEngine.
    pub fn new(
        store: Arc<PostgresMemoryStore>,
        config: RecallConfig,
        extraction_enabled: bool,
    ) -> Self {
        Self {
            store,
            config,
            extraction_enabled,
        }
    }

    /// Execute recall: find relevant memories, deduplicate by session, bump salience.
    ///
    /// # Arguments
    /// - `query_embedding`: Vector embedding of the user's query (required).
    /// - `session_id`: Optional session identifier. Auto-generated if None.
    /// - `reset`: If true, clears session recall history before querying.
    /// - `workspace`: Optional workspace scope. When Some, returns workspace-scoped + global memories.
    ///
    /// # Returns
    /// A `RecallResult` with session_id, count, memories, and summary=None (query-based path).
    /// Empty result (count=0, memories=[]) is valid — not an error.
    pub async fn recall(
        &self,
        query_embedding: &[f32],
        session_id: Option<String>,
        reset: bool,
        workspace: Option<&str>,
    ) -> Result<RecallResult, MemcpError> {
        // a. Resolve session_id: generate if not provided.
        let session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        // b. Ensure session row exists (idempotent upsert).
        self.store.ensure_session(&session_id).await?;

        // c. Reset session dedup history if requested.
        if reset {
            self.store.clear_session_recalls(&session_id).await?;
        }

        // d. Execute tiered recall query with session dedup exclusion.
        let candidates = self
            .store
            .recall_candidates(
                query_embedding,
                &session_id,
                self.config.min_relevance,
                self.config.max_memories,
                self.extraction_enabled,
                workspace,
            )
            .await?;

        // e. Build result and record each recalled memory in the session.
        let mut memories = Vec::with_capacity(candidates.len());
        for (memory_id, content, relevance) in &candidates {
            self.store
                .insert_session_recall(&session_id, memory_id, *relevance)
                .await?;
            memories.push(RecalledMemory {
                memory_id: memory_id.clone(),
                content: content.clone(),
                relevance: *relevance,
            });
        }

        // f. Fire-and-forget salience bump for all recalled memories.
        if !memories.is_empty() {
            let store_clone = Arc::clone(&self.store);
            let ids: Vec<String> = memories.iter().map(|r| r.memory_id.clone()).collect();
            let multiplier = self.config.bump_multiplier;
            let ceiling = self.config.stability_ceiling;
            tokio::task::spawn(async move {
                for id in ids {
                    let _ = store_clone
                        .recall_bump_salience(&id, multiplier, ceiling)
                        .await;
                }
            });
        }

        // g. Return result. summary=None for the query-based path (backward compatible).
        let count = memories.len();
        Ok(RecallResult {
            session_id,
            count,
            memories,
            summary: None,
        })
    }

    /// Execute query-less recall: rank memories by salience without vector search.
    ///
    /// Designed for cold-start context injection — the agent has no user query yet
    /// but wants to load relevant project state before the conversation begins.
    ///
    /// Memories are ranked by stability + recency + access + reinforcement via
    /// `SalienceScorer::rank()` with `rrf_score=0.0` on all hits (semantic dimension
    /// contributes uniformly, dropping out of relative ranking).
    ///
    /// # Arguments
    /// - `session_id`: Optional session identifier. Auto-generated if None.
    /// - `reset`: If true, clears session recall history before querying.
    /// - `workspace`: Optional workspace scope.
    /// - `first`: When true, fetches and pins the most recent `project-summary` tagged
    ///   memory in the `summary` field. Summary is NOT counted toward `count` and NOT
    ///   added to session_recalls (so it reappears on subsequent first-session calls).
    /// - `max_memories_override`: Override for `config.max_memories`. None = use config.
    ///
    /// # Returns
    /// A `RecallResult` with session_id, count, memories, and optionally summary.
    pub async fn recall_queryless(
        &self,
        session_id: Option<String>,
        reset: bool,
        workspace: Option<&str>,
        first: bool,
        max_memories_override: Option<usize>,
    ) -> Result<RecallResult, MemcpError> {
        // a. Resolve session_id.
        let session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        // b. Ensure session row exists.
        self.store.ensure_session(&session_id).await?;

        // c. Reset if requested.
        if reset {
            self.store.clear_session_recalls(&session_id).await?;
        }

        // d. Fetch project summary (only when first=true).
        //    Not added to session_recalls — pinned summary always reappears.
        let summary = if first {
            match self.store.fetch_project_summary(workspace).await? {
                Some((id, content)) => Some(RecalledMemory {
                    memory_id: id,
                    content,
                    relevance: 1.0, // pinned, not relevance-ranked
                }),
                None => None,
            }
        } else {
            None
        };

        // e. Fetch queryless candidates — overfetch 3x for salience re-ranking.
        let max_memories = max_memories_override.unwrap_or(self.config.max_memories);
        let overfetch = max_memories * 3;
        let candidates = self
            .store
            .recall_candidates_queryless(&session_id, overfetch, workspace)
            .await?;

        // f. Build ScoredHit + SalienceInput vectors for salience scoring.
        //    Fields not available from the queryless query use sensible defaults.
        let mut hits: Vec<ScoredHit> = Vec::with_capacity(candidates.len());
        let mut salience_inputs: Vec<SalienceInput> = Vec::with_capacity(candidates.len());

        for candidate in &candidates {
            let memory = Memory {
                id: candidate.memory_id.clone(),
                content: candidate.content.clone(),
                type_hint: "unknown".to_string(),
                source: "unknown".to_string(),
                tags: candidate.tags.clone(),
                created_at: candidate.updated_at, // approximate — OK for scoring
                updated_at: candidate.updated_at,
                last_accessed_at: None,
                access_count: candidate.access_count,
                embedding_status: "complete".to_string(), // already filtered
                extracted_entities: None,
                extracted_facts: None,
                extraction_status: "skipped".to_string(),
                is_consolidated_original: false,
                consolidated_into: None,
                actor: None,
                actor_type: "unknown".to_string(),
                audience: "global".to_string(),
                parent_id: None,
                chunk_index: None,
                total_chunks: None,
                event_time: None,
                event_time_precision: None,
                workspace: workspace.map(String::from),
            };

            hits.push(ScoredHit {
                memory,
                rrf_score: 0.0, // no semantic signal — drops out via normalize()
                salience_score: 0.0,
                match_source: "queryless".to_string(),
                breakdown: None,
                composite_score: 0.0,
            });

            let days_since_reinforced = candidate
                .last_reinforced_at
                .map(|ts| {
                    let dur = Utc::now().signed_duration_since(ts);
                    (dur.num_seconds() as f64 / 86_400.0).max(0.0)
                })
                .unwrap_or(365.0);

            salience_inputs.push(SalienceInput {
                stability: candidate.stability,
                days_since_reinforced,
            });
        }

        // g. Run SalienceScorer::rank() and truncate to max_memories.
        //    SalienceConfig::default() is correct — rrf_score=0.0 means normalize([0,...])
        //    = [1,...] making the semantic dimension a uniform constant that drops out.
        let salience_config = SalienceConfig::default();
        let scorer = SalienceScorer::new(&salience_config);
        scorer.rank(&mut hits, &salience_inputs);
        hits.truncate(max_memories);

        // h. Record session recalls and build memories vec.
        //    ONLY for `memories` — NOT for `summary` (Pitfall 1: summary must not be deduped).
        let mut memories = Vec::with_capacity(hits.len());
        for hit in &hits {
            self.store
                .insert_session_recall(&session_id, &hit.memory.id, hit.salience_score as f32)
                .await?;
            memories.push(RecalledMemory {
                memory_id: hit.memory.id.clone(),
                content: hit.memory.content.clone(),
                relevance: hit.salience_score as f32,
            });
        }

        // i. Fire-and-forget salience bump (same pattern as recall()).
        if !memories.is_empty() {
            let store_clone = Arc::clone(&self.store);
            let ids: Vec<String> = memories.iter().map(|r| r.memory_id.clone()).collect();
            let multiplier = self.config.bump_multiplier;
            let ceiling = self.config.stability_ceiling;
            tokio::task::spawn(async move {
                for id in ids {
                    let _ = store_clone
                        .recall_bump_salience(&id, multiplier, ceiling)
                        .await;
                }
            });
        }

        // j. Return result. count = memories.len() — does NOT include summary.
        let count = memories.len();
        Ok(RecallResult {
            session_id,
            count,
            memories,
            summary,
        })
    }
}
