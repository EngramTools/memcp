//! POST /v1/memory/span handler + shared `compute_memory_span` helper (Phase 24.75 Plan 04).
//!
//! Delivers CHUNK-04: runtime topic-span drill-down. Splits the target memory at
//! query time, embeds each candidate span, and returns the span semantically closest
//! to `topic` with byte offsets into the parent memory's `content`.
//!
//! Shared across MCP / HTTP / CLI surfaces — each entry point constructs the shared
//! dependencies and delegates to `compute_memory_span`. This mirrors Phase 24.5's
//! `run_ingest_batch_with_ctx` pattern: one code path, no triple-duplicated logic.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::embedding::EmbeddingProvider;
use crate::errors::MemcpError;
use crate::pipeline::chunking::splitter::split_sentences;
use crate::store::MemoryStore;
use crate::transport::health::AppState;

/// Request body for `POST /v1/memory/span` and shared compute entry.
#[derive(Debug, Deserialize, Serialize)]
pub struct MemorySpanRequest {
    pub memory_id: String,
    pub topic: String,
}

/// Byte offset range `[start, end)` into `Memory::content`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpanBytes {
    pub start: usize,
    pub end: usize,
}

/// Response body — byte-identical across MCP / HTTP / CLI (D-04 invariant).
#[derive(Debug, Serialize, Deserialize)]
pub struct MemorySpanResponse {
    pub content: String,
    pub span: SpanBytes,
}

/// Topic-embedding cache keyed by exact topic string. Bounded to 100 entries; when
/// full, drops an arbitrary existing entry (HashMap iter order is unspecified — good
/// enough for v1 per RESEARCH Don't-Hand-Roll row). Shared across surfaces so the
/// same topic hit on HTTP and MCP reuses one embedding call.
pub type TopicEmbeddingCache = Arc<Mutex<HashMap<String, Vec<f32>>>>;

pub const TOPIC_CACHE_MAX: usize = 100;
pub const TOPIC_MAX_LEN: usize = 512;
pub const MAX_SPANS_PER_MEMORY: usize = 64;
/// Chunking config baked into get_memory_span. Matches D-04 rationale — these are
/// query-time defaults independent of any store-time ChunkingConfig (which is now
/// vestigial post-24.75). Hard-coded so the tool behaves identically regardless of
/// operator config.
pub const SPAN_MAX_CHARS: usize = 2048;
pub const SPAN_OVERLAP_SENTENCES: usize = 1;
pub const SPAN_MIN_CONTENT_CHARS: usize = 256;

/// Cosine similarity between two equal-length vectors. Returns 0.0 on mismatched /
/// zero-norm inputs to avoid NaN propagation through best-score comparisons.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Shared compute entry. MCP / HTTP / CLI all delegate here.
///
/// Threat-model mitigations applied:
///   - T-24.75-04-01: bail if topic > TOPIC_MAX_LEN.
///   - T-24.75-04-02: cap candidate spans to MAX_SPANS_PER_MEMORY.
///   - T-24.75-04-03: uniform "memory not found" error message regardless of why.
///   - T-24.75-04-05: topic string NOT logged (emit only tool + memory_id in spans).
pub async fn compute_memory_span(
    store: Arc<dyn MemoryStore + Send + Sync>,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    topic_cache: TopicEmbeddingCache,
    memory_id: &str,
    topic: &str,
) -> Result<MemorySpanResponse, MemcpError> {
    if topic.len() > TOPIC_MAX_LEN {
        return Err(MemcpError::Validation {
            message: format!("topic exceeds {} chars", TOPIC_MAX_LEN),
            field: Some("topic".to_string()),
        });
    }

    // MemcpError::NotFound is surfaced as-is; callers map to 404.
    let memory = store.get(memory_id).await?;

    // Topic embedding: cache lookup → provider.embed on miss → best-effort insert.
    let topic_emb = {
        let cache = topic_cache.lock().await;
        if let Some(v) = cache.get(topic) {
            v.clone()
        } else {
            // Drop lock while embedding to avoid serializing all callers on the mutex.
            drop(cache);
            let v = provider
                .embed(topic)
                .await
                .map_err(|e| MemcpError::Internal(format!("topic embed failed: {}", e)))?;
            let mut cache = topic_cache.lock().await;
            if !cache.contains_key(topic) {
                if cache.len() >= TOPIC_CACHE_MAX {
                    // HashMap iter order is arbitrary — acceptable bound, simple drop.
                    if let Some(k) = cache.keys().next().cloned() {
                        cache.remove(&k);
                    }
                }
                cache.insert(topic.to_string(), v.clone());
            }
            v
        }
    };

    // Split on sentence boundaries via the retained runtime utility. Returns empty
    // when the content fits in a single span — in that case the whole memory IS the
    // span (start=0, end=len).
    let groups = split_sentences(&memory.content, SPAN_MAX_CHARS, SPAN_OVERLAP_SENTENCES);

    if groups.is_empty() || memory.content.len() < SPAN_MIN_CONTENT_CHARS {
        return Ok(MemorySpanResponse {
            content: memory.content.clone(),
            span: SpanBytes {
                start: 0,
                end: memory.content.len(),
            },
        });
    }

    // Cap candidate groups to MAX_SPANS_PER_MEMORY (T-24.75-04-02).
    let mut groups = groups;
    if groups.len() > MAX_SPANS_PER_MEMORY {
        groups.truncate(MAX_SPANS_PER_MEMORY);
    }

    // Assemble span bodies (sentences joined) — used for embedding + as the
    // ranking input. The returned content is the precise original substring, so
    // callers get byte-accurate offsets.
    let span_bodies: Vec<String> = groups
        .iter()
        .map(|sentences| sentences.join(""))
        .collect();

    // Embed each span and pick the top cosine match against the topic embedding.
    let mut best_idx: usize = 0;
    let mut best_score: f32 = f32::NEG_INFINITY;
    for (i, body) in span_bodies.iter().enumerate() {
        let emb = provider
            .embed(body)
            .await
            .map_err(|e| MemcpError::Internal(format!("span embed failed: {}", e)))?;
        let score = cosine(&topic_emb, &emb);
        if score > best_score {
            best_score = score;
            best_idx = i;
        }
    }

    // Compute byte offsets by anchoring the first + last sentence of the winning
    // group back to the original content. `unicode_sentences` consumes inter-
    // sentence whitespace — re-anchoring preserves the parent's exact bytes so
    // callers' `content[start..end] == returned.content` invariant holds.
    let winning_sentences = &groups[best_idx];
    let first_sentence = winning_sentences
        .first()
        .map(|s| s.as_str())
        .unwrap_or("");
    let last_sentence = winning_sentences
        .last()
        .map(|s| s.as_str())
        .unwrap_or("");

    let start = memory.content.find(first_sentence).unwrap_or(0);
    // `rfind` guards against repeating-body false hits on the start sentence.
    let last_start = memory.content[start..]
        .find(last_sentence)
        .map(|rel| start + rel)
        .unwrap_or(start);
    let end = (last_start + last_sentence.len()).min(memory.content.len());
    let end = end.max(start);

    // Return the exact parent substring so offsets are guaranteed byte-accurate.
    let content = memory.content[start..end].to_string();

    Ok(MemorySpanResponse {
        content,
        span: SpanBytes { start, end },
    })
}

/// POST /v1/memory/span
pub async fn memory_span_handler(
    State(state): State<AppState>,
    Json(req): Json<MemorySpanRequest>,
) -> impl IntoResponse {
    let store = match state.store.as_ref() {
        Some(s) => (s.clone()) as Arc<dyn MemoryStore + Send + Sync>,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "store not available"})),
            )
                .into_response();
        }
    };
    let provider = match state.embed_provider.as_ref() {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "embedding_provider_unavailable"})),
            )
                .into_response();
        }
    };
    tracing::info!(
        tool = "get_memory_span",
        memory_id = %req.memory_id,
        "HTTP /v1/memory/span"
    );
    match compute_memory_span(
        store,
        provider,
        state.topic_embedding_cache.clone(),
        &req.memory_id,
        &req.topic,
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response(),
        Err(MemcpError::NotFound { .. }) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "memory not found"})),
        )
            .into_response(),
        Err(MemcpError::Validation { message, .. }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": message})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
