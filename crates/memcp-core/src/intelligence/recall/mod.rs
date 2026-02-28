//! Recall engine — automatic context injection with session-scoped dedup.
//!
//! `RecallEngine::recall()` implements the tiered recall strategy:
//! - Extraction enabled: query against extracted_facts (compact fact content).
//! - Extraction disabled: filter to type_hint IN (fact, summary) memories.
//!
//! Session dedup: memories recalled within a session are not re-injected.
//! Implicit salience bump: recalled memories get a lightweight stability boost
//! (x1.15 by default, lighter than explicit reinforce at x1.5).

use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use crate::config::RecallConfig;
use crate::errors::MemcpError;
use crate::store::postgres::PostgresMemoryStore;

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
    /// A `RecallResult` with session_id, count, and memories slice.
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

        // g. Return result.
        let count = memories.len();
        Ok(RecallResult {
            session_id,
            count,
            memories,
        })
    }
}
