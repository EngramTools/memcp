/// LoCoMo ingestion: converts conversation sessions into memcp memories.
///
/// Supports two modes:
/// - PerTurn: one memory per dialog turn
/// - PerSession: one memory per session (concatenated turns)
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};

use crate::embedding::pipeline::EmbeddingPipeline;
use crate::embedding::{build_embedding_text, EmbeddingJob};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{CreateMemory, MemoryStore};

use super::{LoCoMoIngestionMode, LoCoMoSample};

/// Ingest a LoCoMo sample into memcp memories.
///
/// Returns the number of memories created.
pub async fn ingest_sample(
    sample: &LoCoMoSample,
    mode: &LoCoMoIngestionMode,
    store: &Arc<PostgresMemoryStore>,
    pipeline: &EmbeddingPipeline,
) -> Result<usize> {
    let count = match mode {
        LoCoMoIngestionMode::PerTurn => ingest_per_turn(sample, store, pipeline).await?,
        LoCoMoIngestionMode::PerSession => ingest_per_session(sample, store, pipeline).await?,
    };

    pipeline.flush().await;
    Ok(count)
}

/// Ingest one memory per dialog turn.
async fn ingest_per_turn(
    sample: &LoCoMoSample,
    store: &Arc<PostgresMemoryStore>,
    pipeline: &EmbeddingPipeline,
) -> Result<usize> {
    let mut count = 0;

    for (session_idx, session) in sample.conversation.iter().enumerate() {
        let session_base_time = parse_locomo_date(&session.date);

        for (turn_idx, turn) in session.dialog.iter().enumerate() {
            let content = format!("{}: {}", turn.speaker, turn.text);

            // Offset each turn by 1 second within the session to preserve ordering.
            let created_at = session_base_time
                .map(|t| t + Duration::seconds(turn_idx as i64));

            let memory = CreateMemory {
                content,
                type_hint: "conversation".to_string(),
                source: "locomo-benchmark".to_string(),
                tags: Some(vec![
                    "locomo".to_string(),
                    format!("session:{}", session_idx),
                    format!("turn:{}", turn.dialog_id),
                ]),
                created_at,
                actor: None,
                actor_type: "system".to_string(),
                audience: "global".to_string(),
                idempotency_key: None,
                parent_id: None,
                chunk_index: None,
                total_chunks: None,
                event_time: None,
                event_time_precision: None,
                workspace: None,
            };

            let stored = store.store(memory).await?;
            let text = build_embedding_text(&stored.content, &stored.tags);
            pipeline.enqueue(EmbeddingJob {
                memory_id: stored.id,
                text,
                attempt: 0,
                completion_tx: None,
                tier: "fast".to_string(),
            });

            count += 1;
        }
    }

    Ok(count)
}

/// Ingest one memory per conversation session (concatenated turns).
async fn ingest_per_session(
    sample: &LoCoMoSample,
    store: &Arc<PostgresMemoryStore>,
    pipeline: &EmbeddingPipeline,
) -> Result<usize> {
    let mut count = 0;

    for (session_idx, session) in sample.conversation.iter().enumerate() {
        let session_base_time = parse_locomo_date(&session.date);

        // Build concatenated content for the full session.
        let mut lines = vec![format!("Session date: {}", session.date)];
        for turn in &session.dialog {
            lines.push(format!("{}: {}", turn.speaker, turn.text));
        }
        let content = lines.join("\n");

        let memory = CreateMemory {
            content,
            type_hint: "conversation".to_string(),
            source: "locomo-benchmark".to_string(),
            tags: Some(vec![
                "locomo".to_string(),
                format!("session:{}", session_idx),
            ]),
            created_at: session_base_time,
            actor: None,
            actor_type: "system".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            workspace: None,
        };

        let stored = store.store(memory).await?;
        let text = build_embedding_text(&stored.content, &stored.tags);
        pipeline.enqueue(EmbeddingJob {
            memory_id: stored.id,
            text,
            attempt: 0,
            completion_tx: None,
            tier: "fast".to_string(),
        });

        count += 1;
    }

    Ok(count)
}

/// Parse a LoCoMo date string to UTC midnight.
///
/// Supports formats:
/// - "March 15, 2023" (long month name)
/// - "2023-03-15" (ISO 8601)
/// - "03/15/2023" (US slash format)
pub fn parse_locomo_date(date_str: &str) -> Option<DateTime<Utc>> {
    // Try ISO 8601 first.
    if let Ok(d) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|dt| Utc.from_utc_datetime(&dt));
    }
    // Try "Month DD, YYYY" (e.g., "March 15, 2023").
    if let Ok(d) = NaiveDate::parse_from_str(date_str, "%B %d, %Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| Utc.from_utc_datetime(&dt));
    }
    // Try "Mon DD, YYYY" (abbreviated month name).
    if let Ok(d) = NaiveDate::parse_from_str(date_str, "%b %d, %Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| Utc.from_utc_datetime(&dt));
    }
    // Try US slash format "MM/DD/YYYY".
    if let Ok(d) = NaiveDate::parse_from_str(date_str, "%m/%d/%Y") {
        return d.and_hms_opt(0, 0, 0).map(|dt| Utc.from_utc_datetime(&dt));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_locomo_date_long_month() {
        let dt = parse_locomo_date("March 15, 2023");
        assert!(dt.is_some(), "Should parse long month name");
        let dt = dt.unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2023-03-15");
    }

    #[test]
    fn test_parse_locomo_date_iso() {
        let dt = parse_locomo_date("2023-03-15");
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().format("%Y-%m-%d").to_string(), "2023-03-15");
    }

    #[test]
    fn test_parse_locomo_date_slash() {
        let dt = parse_locomo_date("03/15/2023");
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().format("%Y-%m-%d").to_string(), "2023-03-15");
    }

    #[test]
    fn test_parse_locomo_date_invalid() {
        let dt = parse_locomo_date("not a date");
        assert!(dt.is_none());
    }
}
