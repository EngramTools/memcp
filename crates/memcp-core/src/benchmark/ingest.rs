//! Benchmark ingestion logic: converts LongMemEval sessions into stored memories.
//!
//! Each turn in a session becomes an individual memory with:
//! - session_id tag for grouping
//! - turn index tag for ordering
//! - role tag for filtering
//! - created_at override from haystack_dates for temporal reasoning accuracy

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::embedding::pipeline::EmbeddingPipeline;
use crate::embedding::{build_embedding_text, EmbeddingJob};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{CreateMemory, MemoryStore};

use super::dataset::LongMemEvalQuestion;

/// Ingest a question's haystack sessions as individual turn-level memories.
///
/// Each turn becomes a separate memory tagged with session_id and turn_index.
/// created_at is set from haystack_dates for temporal reasoning accuracy.
/// After ingestion, flushes the embedding pipeline to ensure vectors are ready for search.
///
/// Returns the total number of turns ingested.
pub async fn ingest_question(
    question: &LongMemEvalQuestion,
    store: &Arc<PostgresMemoryStore>,
    pipeline: &EmbeddingPipeline,
) -> Result<usize, anyhow::Error> {
    let mut turn_count = 0;

    for (session_idx, session) in question.haystack_sessions.iter().enumerate() {
        // Parse session date from haystack_dates
        let session_date = if session_idx < question.haystack_dates.len() {
            parse_session_date(&question.haystack_dates[session_idx])
        } else {
            None
        };

        let session_id = if session_idx < question.haystack_session_ids.len() {
            question.haystack_session_ids[session_idx].as_str()
        } else {
            "unknown"
        };

        for (turn_idx, turn) in session.iter().enumerate() {
            // Format: "[role] content" for embedding quality
            let content = format!("[{}] {}", turn.role, turn.content);

            let memory = CreateMemory {
                content,
                type_hint: "conversation".to_string(),
                source: format!("benchmark:session-{}", session_id),
                tags: Some(vec![
                    format!("session:{}", session_id),
                    format!("turn:{}", turn_idx),
                    format!("role:{}", turn.role),
                ]),
                created_at: session_date,
                actor: None,
                actor_type: "system".to_string(),
                audience: "global".to_string(),
                idempotency_key: None,
                parent_id: None,
                chunk_index: None,
                total_chunks: None,
                event_time: None,
                event_time_precision: None,
                project: None,
                trust_level: None,
                session_id: None,
                agent_role: None,
                write_path: None,
                knowledge_tier: None,
                source_ids: None,
            };

            let stored = store.store(memory).await?;

            // Enqueue embedding job
            let text = build_embedding_text(
                &stored.content,
                stored.abstract_text.as_deref(),
                &stored.tags,
            );
            pipeline.enqueue(EmbeddingJob {
                memory_id: stored.id,
                text,
                attempt: 0,
                completion_tx: None,
                tier: "fast".to_string(),
            });

            turn_count += 1;
        }
    }

    // Wait for all embeddings to complete before returning
    pipeline.flush().await;

    Ok(turn_count)
}

/// Parse a session date string into a DateTime<Utc>.
///
/// Supports two formats:
/// - `"2023/05/20 (Sat) 02:21"` — actual LongMemEval dataset format (preserves time)
/// - `"2023-05-15"` — fallback for test fixtures (uses noon UTC)
fn parse_session_date(date_str: &str) -> Option<chrono::DateTime<Utc>> {
    // Primary: actual dataset format with day-of-week and time
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y/%m/%d (%a) %H:%M") {
        return Some(Utc.from_utc_datetime(&dt));
    }
    // Fallback: simple date format for test fixtures
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .map(|dt| Utc.from_utc_datetime(&dt))
}
