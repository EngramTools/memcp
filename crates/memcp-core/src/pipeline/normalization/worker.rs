//! Async normalization worker: bounded mpsc channel + background tokio task.
//!
//! Non-blocking design: callers enqueue jobs without waiting for completion.
//! Failed normalizations are retried up to 3 times with exponential backoff
//! (1s, 2s, 4s), then marked as failed.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::{NormalizationJob, NormalizationResult};
use crate::pipeline::extraction::StructuredFact;
use crate::store::postgres::PostgresMemoryStore;

/// Async normalization pipeline: enqueues jobs onto a bounded mpsc channel and
/// processes them in a background tokio task.
pub struct NormalizationWorker {
    sender: mpsc::Sender<NormalizationJob>,
}

impl NormalizationWorker {
    /// Create a new NormalizationWorker and spawn the background worker.
    ///
    /// - `store`: The PostgresMemoryStore for reading/writing entity graph data.
    /// - `capacity`: Bounded channel capacity (recommended: 500).
    pub fn new(store: Arc<PostgresMemoryStore>, capacity: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<NormalizationJob>(capacity);
        let retry_tx = tx.clone();

        tokio::spawn(async move {
            // Periodic backfill sweep every 5 minutes — picks up any jobs that were
            // dropped when the channel was full, or that arrived before the worker started.
            let mut backfill_interval =
                tokio::time::interval(Duration::from_secs(5 * 60));

            loop {
                tokio::select! {
                    maybe_job = rx.recv() => {
                        let Some(job) = maybe_job else { break };
                        match process_job(&store, &job).await {
                            Ok(result) => {
                                if let Err(e) = store
                                    .update_normalization_status(&job.memory_id, "complete")
                                    .await
                                {
                                    tracing::error!(
                                        memory_id = %job.memory_id,
                                        error = %e,
                                        "Failed to mark normalization complete"
                                    );
                                } else {
                                    tracing::debug!(
                                        memory_id = %job.memory_id,
                                        entities_resolved = result.entities_resolved,
                                        mentions_created = result.mentions_created,
                                        facts_stored = result.facts_stored,
                                        "Normalization complete"
                                    );
                                }
                            }
                            Err(e) if job.attempt < 3 => {
                                tracing::warn!(
                                    memory_id = %job.memory_id,
                                    attempt = job.attempt + 1,
                                    error = %e,
                                    "Normalization failed, retrying"
                                );
                                // Exponential backoff: 1s, 2s, 4s
                                let delay = Duration::from_secs(2u64.pow(job.attempt as u32));
                                tokio::time::sleep(delay).await;
                                // Use blocking send on retry to ensure the job is not silently
                                // dropped when the channel is full.
                                if retry_tx.send(NormalizationJob {
                                    attempt: job.attempt + 1,
                                    ..job
                                }).await.is_err() {
                                    tracing::warn!("Normalization channel closed during retry — dropping job");
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    memory_id = %job.memory_id,
                                    attempts = 3,
                                    error = %e,
                                    "Normalization failed after 3 retries, marking as failed"
                                );
                                let _ = store
                                    .update_normalization_status(&job.memory_id, "failed")
                                    .await;
                            }
                        }
                    }
                    _ = backfill_interval.tick() => {
                        // Process pending jobs directly (bypass channel) to recover from
                        // any drops that occurred when the channel was full.
                        match store.get_pending_normalization(50).await {
                            Ok(memories) if !memories.is_empty() => {
                                tracing::info!(count = memories.len(), "Normalization backfill sweep");
                                for memory in memories {
                                    let (facts, structured_facts) =
                                        parse_facts_jsonb(memory.extracted_facts.as_ref());
                                    let job = NormalizationJob {
                                        memory_id: memory.id,
                                        extracted_entities: extract_strings_from_jsonb(memory.extracted_entities.as_ref()),
                                        extracted_facts: facts,
                                        structured_facts,
                                        content: memory.content,
                                        attempt: 0,
                                    };
                                    match process_job(&store, &job).await {
                                        Ok(result) => {
                                            tracing::debug!(
                                                memory_id = %job.memory_id,
                                                entities_resolved = result.entities_resolved,
                                                "Backfill normalization complete"
                                            );
                                            let _ = store
                                                .update_normalization_status(&job.memory_id, "complete")
                                                .await;
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                memory_id = %job.memory_id,
                                                error = %e,
                                                "Backfill normalization failed — will retry next sweep"
                                            );
                                        }
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to fetch pending normalization memories during backfill");
                            }
                        }
                    }
                }
            }
        });

        NormalizationWorker { sender: tx }
    }

    /// Enqueue a normalization job (non-blocking).
    ///
    /// Uses try_send — if the channel is full, the job is dropped and a warning is logged.
    /// The backfill process will pick up missed memories on next startup.
    pub fn enqueue(&self, job: NormalizationJob) {
        if self.sender.try_send(job).is_err() {
            tracing::warn!(
                "Normalization queue full — memory stored, normalization deferred to backfill"
            );
        }
    }

    /// Return a clone of the underlying mpsc sender.
    pub fn sender(&self) -> mpsc::Sender<NormalizationJob> {
        self.sender.clone()
    }

    /// Backfill: fetch pending memories from the store and enqueue them.
    ///
    /// Reads up to `limit` memories where `entity_normalization_status = 'pending'`
    /// and `extraction_status = 'complete'`, then enqueues a job for each.
    pub async fn process_pending(&self, store: &PostgresMemoryStore, limit: i64) {
        match store.get_pending_normalization(limit).await {
            Ok(memories) => {
                let count = memories.len();
                for memory in memories {
                    let entities = extract_strings_from_jsonb(memory.extracted_entities.as_ref());
                    let (facts, structured_facts) =
                        parse_facts_jsonb(memory.extracted_facts.as_ref());
                    self.enqueue(NormalizationJob {
                        memory_id: memory.id,
                        extracted_entities: entities,
                        extracted_facts: facts,
                        structured_facts,
                        content: memory.content,
                        attempt: 0,
                    });
                }
                tracing::info!(count, "Normalization backfill enqueued");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to fetch pending normalization memories");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Core processing logic
// ---------------------------------------------------------------------------

async fn process_job(
    store: &PostgresMemoryStore,
    job: &NormalizationJob,
) -> Result<NormalizationResult, crate::errors::MemcpError> {
    let mut entities_resolved = 0usize;
    let mut mentions_created = 0usize;
    let mut facts_stored = 0usize;

    for raw in &job.extracted_entities {
        let normalized = raw.trim().to_lowercase();
        if normalized.is_empty() {
            continue;
        }

        // Check whether a canonical entity already exists.
        let entity = match store.find_entity_by_name(&normalized).await? {
            Some(existing) => existing,
            None => {
                // Infer entity type from the original (pre-lowercase) string.
                // Heuristic: if the first character is uppercase → person or place,
                // otherwise treat as concept. This is deliberately simple — LLM-quality
                // classification is out of scope for the normalization worker.
                let entity_type = infer_entity_type(raw.trim());
                store.upsert_entity(&normalized, entity_type, &[]).await?
            }
        };
        entities_resolved += 1;

        // Build a short context snippet from the memory content (first 200 chars).
        let snippet = if job.content.len() > 200 {
            Some(&job.content[..200])
        } else {
            Some(job.content.as_str())
        };

        store
            .create_mention(entity.id, &job.memory_id, snippet)
            .await?;
        mentions_created += 1;
    }

    // Prefer structured facts (entity-linked) over the flat fallback.
    if !job.structured_facts.is_empty() {
        for sf in &job.structured_facts {
            let entity_name = sf.entity.trim().to_lowercase();
            if entity_name.is_empty() {
                continue;
            }
            let entity_id = match store.find_entity_by_name(&entity_name).await? {
                Some(e) => e.id,
                None => {
                    tracing::debug!(
                        entity = %entity_name,
                        "Structured fact references unknown entity — skipping"
                    );
                    continue;
                }
            };
            let attribute = sf.attribute.trim().to_lowercase();
            let value = serde_json::Value::String(sf.value.clone());
            store
                .create_fact(entity_id, &attribute, &value, Some(&job.memory_id), 0.7)
                .await?;
            facts_stored += 1;
        }
    } else {
        // Fallback: flat fact strings anchored to the first resolved entity.
        // Parse facts as "attribute: value" pairs when the colon separator is present;
        // otherwise store the whole string under the attribute "note".
        let anchor_entity = job
            .extracted_entities
            .first()
            .map(|s| s.trim().to_lowercase());

        let entity_id: Option<uuid::Uuid> = if let Some(ref name) = anchor_entity {
            store.find_entity_by_name(name).await?.map(|e| e.id)
        } else {
            None
        };

        if let Some(entity_id) = entity_id {
            for raw_fact in &job.extracted_facts {
                let trimmed = raw_fact.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let (attribute, value) = parse_fact(trimmed);
                store
                    .create_fact(entity_id, &attribute, &value, Some(&job.memory_id), 0.7)
                    .await?;
                facts_stored += 1;
            }
        }
    }

    Ok(NormalizationResult {
        entities_resolved,
        mentions_created,
        facts_stored,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// TODO: Replace with LLM-based entity type classification
/// Infer a coarse entity type from the raw (pre-normalized) entity string.
///
/// Capitalised first letter → "person" (most common proper noun in agent memories).
/// All lowercase or mixed → "concept".
fn infer_entity_type(raw: &str) -> &'static str {
    let first_char = raw.chars().next();
    match first_char {
        Some(c) if c.is_uppercase() => "person",
        _ => "concept",
    }
}

/// Parse a raw fact string into an (attribute, JSON value) pair.
///
/// Accepts two formats:
/// - `"attribute: value"` — splits on the first colon.
/// - Anything else → attribute = "note", value = the full string.
fn parse_fact(raw: &str) -> (String, serde_json::Value) {
    if let Some(colon_pos) = raw.find(':') {
        let attribute = raw[..colon_pos].trim().to_lowercase();
        let value_str = raw[colon_pos + 1..].trim();
        (attribute, serde_json::Value::String(value_str.to_string()))
    } else {
        (
            "note".to_string(),
            serde_json::Value::String(raw.to_string()),
        )
    }
}

/// Convert an optional JSONB array value into a `Vec<String>`.
///
/// Silently skips non-string elements and handles a missing/null column.
fn extract_strings_from_jsonb(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Parse the extracted_facts JSONB column into flat strings and structured facts.
///
/// When the column contains an array of objects with `entity`/`attribute`/`value` keys
/// (new structured format), returns them as `structured_facts` and leaves `flat` empty.
/// When the column contains a flat array of strings (old format), returns them in `flat`
/// with an empty `structured_facts`. Either way the caller always gets usable data.
fn parse_facts_jsonb(value: Option<&serde_json::Value>) -> (Vec<String>, Vec<StructuredFact>) {
    match value {
        Some(serde_json::Value::Array(arr)) if !arr.is_empty() => {
            // Attempt structured deserialization first.
            // serde_json can deserialize a &[Value] directly into Vec<StructuredFact>
            // if every element has the required fields.
            if let Ok(structured) =
                serde_json::from_value::<Vec<StructuredFact>>(serde_json::Value::Array(arr.clone()))
            {
                // Only treat as structured if all entries deserialized successfully.
                // serde_json::from_value on a Vec succeeds even if the inner type has
                // #[serde(default)], so guard against accidentally treating flat
                // strings-as-objects by checking entity is non-empty.
                if structured.iter().all(|sf| !sf.entity.is_empty()) {
                    return (Vec::new(), structured);
                }
            }
            // Fallback: flat string array.
            let flat = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            (flat, Vec::new())
        }
        _ => (Vec::new(), Vec::new()),
    }
}
