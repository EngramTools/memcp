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
/// boost_applied and boost_score are populated by Plan 02 when tag-affinity boost is active.
/// They default to false/0.0 and are omitted from JSON output when unset.
#[derive(Debug, Clone, Serialize)]
pub struct RecalledMemory {
    pub memory_id: String,
    pub content: String,
    pub relevance: f32,
    /// Whether any tag boost (explicit or session) was applied to this memory.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub boost_applied: bool,
    /// Total boost score added (explicit + session). 0.0 when no boost.
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub boost_score: f32,
    /// Trust level from the stored memory (0.0–1.0). Skipped in JSON when 1.0 (default).
    #[serde(skip_serializing_if = "is_default_trust")]
    pub trust_level: f32,
    /// Concise abstract of the memory (depth=0 tier). None when abstraction hasn't run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    /// Structured overview of the memory (depth=1 tier). None when abstraction hasn't run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview_text: Option<String>,
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

/// Skip serializing trust_level when it's the default (1.0) — keeps JSON clean for trusted memories.
fn is_default_trust(v: &f32) -> bool {
    (*v - 1.0).abs() < f32::EPSILON
}

/// Prefix-aware tag matching. If boost_tag ends with ':', it's a prefix match.
/// Otherwise exact match.
///
/// Examples:
/// - "channel:" matches "channel:devops", "channel:security" (prefix)
/// - "channel:devops" matches only "channel:devops" (exact)
fn tag_matches(boost_tag: &str, memory_tag: &str) -> bool {
    if boost_tag.ends_with(':') {
        memory_tag.starts_with(boost_tag)
    } else {
        memory_tag == boost_tag
    }
}

/// Compute additive tag boost score. Counts distinct boost_tags that match any memory_tag.
///
/// Multiple matching tags stack: 2 matches * weight = 2x weight. Capped at cap.
/// Returns 0.0 immediately when either input is empty.
pub fn compute_tag_boost(
    boost_tags: &[String],
    memory_tags: &[String],
    weight: f64,
    cap: f64,
) -> f64 {
    if boost_tags.is_empty() || memory_tags.is_empty() {
        return 0.0;
    }
    let matches = boost_tags
        .iter()
        .filter(|bt| memory_tags.iter().any(|mt| tag_matches(bt, mt)))
        .count();
    (matches as f64 * weight).min(cap)
}

/// Extract tags from a Memory.tags JSONB field into Vec<String>.
///
/// Reuses the established pattern from annotate_logic / phase 08.8.
/// Returns empty Vec when tags is None or contains no string elements.
pub fn extract_tags(tags: &Option<serde_json::Value>) -> Vec<String> {
    tags.as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
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
    /// - `project`: Optional project scope. When Some, returns project-scoped + global memories.
    /// - `boost_tags`: Explicit boost tags for tag-affinity ranking. Memories sharing these tags
    ///   receive a soft relevance bonus. Prefix matching: "channel:" boosts all "channel:*" tags.
    ///   Pass `&[]` to skip explicit boost (backward compatible).
    ///
    /// # Returns
    /// A `RecallResult` with session_id, count, memories, and summary=None (query-based path).
    /// Empty result (count=0, memories=[]) is valid — not an error.
    pub async fn recall(
        &self,
        query_embedding: &[f32],
        session_id: Option<String>,
        reset: bool,
        project: Option<&str>,
        boost_tags: &[String],
    ) -> Result<RecallResult, MemcpError> {
        // a. Resolve session_id: generate if not provided.
        let session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        // b. Ensure session row exists (idempotent upsert).
        self.store.ensure_session(&session_id).await?;

        // c. Reset session dedup history if requested.
        if reset {
            self.store.clear_session_recalls(&session_id).await?;
        }

        // d. Fetch session tags for implicit topic-affinity boost.
        let session_tags = if self.config.session_topic_tracking {
            self.store
                .get_session_tags(&session_id)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // e. Execute tiered recall query with session dedup exclusion.
        //    recall_candidates now returns (memory_id, content, relevance, tags).
        let candidates = self
            .store
            .recall_candidates(
                query_embedding,
                &session_id,
                self.config.min_relevance,
                self.config.max_memories,
                self.extraction_enabled,
                project,
            )
            .await?;

        // f. Apply boost, build memories, accumulate tags for session topic tracking.
        let mut memories: Vec<RecalledMemory> = Vec::with_capacity(candidates.len());
        let mut accumulated_tags: Vec<String> = Vec::new();

        for candidate in &candidates {
            let memory_tags = extract_tags(&candidate.tags);

            // Compute explicit + implicit boost.
            let explicit_boost = compute_tag_boost(
                boost_tags,
                &memory_tags,
                self.config.tag_boost_weight,
                self.config.tag_boost_cap,
            );
            let implicit_boost = compute_tag_boost(
                &session_tags,
                &memory_tags,
                self.config.session_boost_weight,
                self.config.session_boost_cap,
            );
            let total_boost = explicit_boost + implicit_boost;
            let boosted_relevance = candidate.relevance + total_boost as f32;

            // Apply trust weighting: multiply by trust_level with floor at 0.05
            let trust = candidate.trust_level.max(0.05);
            let trust_weighted = boosted_relevance * trust;

            memories.push(RecalledMemory {
                memory_id: candidate.memory_id.clone(),
                content: candidate.content.clone(),
                relevance: trust_weighted,
                boost_applied: total_boost > 0.0,
                boost_score: total_boost as f32,
                trust_level: candidate.trust_level,
                abstract_text: None,
                overview_text: None,
            });

            // Collect memory tags for session accumulation.
            accumulated_tags.extend(memory_tags);
        }

        // g. Re-sort by boosted relevance DESC (boost may change relative order).
        memories.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // Already capped at max_memories by recall_candidates; no truncate needed.

        // h. Record each recalled memory in the session.
        for mem in &memories {
            self.store
                .insert_session_recall(&session_id, &mem.memory_id, mem.relevance)
                .await?;
        }

        // i. Accumulate session tags (for implicit boost on next recall).
        if self.config.session_topic_tracking && !accumulated_tags.is_empty() {
            accumulated_tags.sort();
            accumulated_tags.dedup();
            let _ = self
                .store
                .accumulate_session_tags(&session_id, &accumulated_tags)
                .await;
        }

        // j. Fire-and-forget salience bump for all recalled memories.
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

        // k. Return result. summary=None for the query-based path (backward compatible).
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
    /// - `project`: Optional project scope.
    /// - `first`: When true, fetches and pins the most recent `project-summary` tagged
    ///   memory in the `summary` field. Summary is NOT counted toward `count` and NOT
    ///   added to session_recalls (so it reappears on subsequent first-session calls).
    /// - `max_memories_override`: Override for `config.max_memories`. None = use config.
    /// - `boost_tags`: Explicit boost tags for tag-affinity ranking. Applied after salience
    ///   scoring, before truncation. Pass `&[]` to skip explicit boost (backward compatible).
    ///
    /// # Returns
    /// A `RecallResult` with session_id, count, memories, and optionally summary.
    pub async fn recall_queryless(
        &self,
        session_id: Option<String>,
        reset: bool,
        project: Option<&str>,
        first: bool,
        max_memories_override: Option<usize>,
        boost_tags: &[String],
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
            self.store
                .fetch_project_summary(project)
                .await?
                .map(|(id, content)| RecalledMemory {
                    memory_id: id,
                    content,
                    relevance: 1.0, // pinned, not relevance-ranked
                    boost_applied: false,
                    boost_score: 0.0,
                    trust_level: 1.0, // summaries are always trusted
                    abstract_text: None,
                    overview_text: None,
                })
        } else {
            None
        };

        // e. Fetch session tags for implicit topic-affinity boost.
        let session_tags = if self.config.session_topic_tracking {
            self.store
                .get_session_tags(&session_id)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // f. Fetch queryless candidates — overfetch 3x for salience re-ranking.
        let max_memories = max_memories_override.unwrap_or(self.config.max_memories);
        let overfetch = max_memories * 3;
        let candidates = self
            .store
            .recall_candidates_queryless(&session_id, overfetch, project)
            .await?;

        // g. Build ScoredHit + SalienceInput vectors for salience scoring.
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
                project: project.map(String::from),
                trust_level: candidate.trust_level,
                session_id: None,
                agent_role: None,
                write_path: None,
                metadata: serde_json::json!({}),
                abstract_text: None,
                overview_text: None,
                abstraction_status: "skipped".to_string(),
                knowledge_tier: "explicit".to_string(),
                source_ids: None,
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

        // h. Run SalienceScorer::rank() — assigns salience_score to each hit.
        //    SalienceConfig::default() is correct — rrf_score=0.0 means normalize([0,...])
        //    = [1,...] making the semantic dimension a uniform constant that drops out.
        let salience_config = SalienceConfig::default();
        let scorer = SalienceScorer::new(&salience_config);
        scorer.rank(&mut hits, &salience_inputs);

        // i. Apply tag-affinity boost AFTER salience scoring, BEFORE truncation.
        //    Boost modifies salience_score in place, then re-sort and truncate.
        if !boost_tags.is_empty() || !session_tags.is_empty() {
            for hit in hits.iter_mut() {
                let memory_tags = extract_tags(&hit.memory.tags);
                let explicit_boost = compute_tag_boost(
                    boost_tags,
                    &memory_tags,
                    self.config.tag_boost_weight,
                    self.config.tag_boost_cap,
                );
                let implicit_boost = compute_tag_boost(
                    &session_tags,
                    &memory_tags,
                    self.config.session_boost_weight,
                    self.config.session_boost_cap,
                );
                hit.salience_score += explicit_boost + implicit_boost;
            }
            // Re-sort after boost.
            hits.sort_by(|a, b| {
                b.salience_score
                    .partial_cmp(&a.salience_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Apply trust weighting to salience scores (after boost, before truncation).
        // Matches the search path pattern: score *= trust.max(0.05).
        for hit in hits.iter_mut() {
            let trust = (hit.memory.trust_level as f64).max(0.05);
            hit.salience_score *= trust;
        }
        // Re-sort after trust weighting.
        hits.sort_by(|a, b| {
            b.salience_score
                .partial_cmp(&a.salience_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        hits.truncate(max_memories);

        // j. Record session recalls and build memories vec.
        //    ONLY for `memories` — NOT for `summary` (Pitfall 1: summary must not be deduped).
        //    Compute boost fields per hit for RecalledMemory.
        let mut memories = Vec::with_capacity(hits.len());
        let mut accumulated_tags: Vec<String> = Vec::new();

        for hit in &hits {
            let memory_tags = extract_tags(&hit.memory.tags);

            // Recompute boost for RecalledMemory fields (cheap, already done above).
            let explicit_boost = compute_tag_boost(
                boost_tags,
                &memory_tags,
                self.config.tag_boost_weight,
                self.config.tag_boost_cap,
            );
            let implicit_boost = compute_tag_boost(
                &session_tags,
                &memory_tags,
                self.config.session_boost_weight,
                self.config.session_boost_cap,
            );
            let total_boost = explicit_boost + implicit_boost;

            self.store
                .insert_session_recall(&session_id, &hit.memory.id, hit.salience_score as f32)
                .await?;
            memories.push(RecalledMemory {
                memory_id: hit.memory.id.clone(),
                content: hit.memory.content.clone(),
                relevance: hit.salience_score as f32,
                boost_applied: total_boost > 0.0,
                boost_score: total_boost as f32,
                trust_level: hit.memory.trust_level,
                abstract_text: hit.memory.abstract_text.clone(),
                overview_text: hit.memory.overview_text.clone(),
            });

            accumulated_tags.extend(memory_tags);
        }

        // k. Accumulate session tags (for implicit boost on next recall).
        if self.config.session_topic_tracking && !accumulated_tags.is_empty() {
            accumulated_tags.sort();
            accumulated_tags.dedup();
            let _ = self
                .store
                .accumulate_session_tags(&session_id, &accumulated_tags)
                .await;
        }

        // l. Fire-and-forget salience bump (same pattern as recall()).
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

        // m. Return result. count = memories.len() — does NOT include summary.
        let count = memories.len();
        Ok(RecallResult {
            session_id,
            count,
            memories,
            summary,
        })
    }
}
