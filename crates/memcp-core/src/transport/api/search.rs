//! POST /v1/search — hybrid search with salience re-ranking.
//!
//! Embeds query in-process via embed_provider when available.
//! Degrades to BM25+symbolic text search when embed_provider is None.

use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde_json::json;

use crate::search::salience::{SalienceInput, SalienceScorer, ScoredHit, dedup_parent_chunks};
use crate::store::{decode_search_keyset_cursor, encode_search_keyset_cursor};
use crate::transport::health::AppState;
use super::types::{SearchRequest, error_json};

/// POST /v1/search
///
/// Executes hybrid search and returns the same JSON envelope as `memcp search --json`.
/// Embeds query in-process when embed_provider is available; degrades to text-only otherwise.
pub async fn search_handler(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !state.ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("daemon not ready")));
    }

    let store = match &state.store {
        Some(s) => s.clone(),
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(error_json("store not available"))),
    };

    // Validate required field
    if req.query.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(error_json("query is required and must not be empty")));
    }

    // Validate min_salience range
    if let Some(ms) = req.min_salience {
        if !(0.0..=1.0).contains(&ms) {
            return (StatusCode::BAD_REQUEST, Json(error_json("min_salience must be between 0.0 and 1.0")));
        }
    }

    let effective_min = req.min_salience
        .or(state.config.search.default_min_salience)
        .unwrap_or(0.0);

    let limit = req.limit as i64;

    // Decode cursor for keyset pagination.
    let cursor_position: Option<(f64, String)> = if let Some(ref c) = req.cursor {
        match decode_search_keyset_cursor(c) {
            Ok(pos) => Some(pos),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(error_json(&format!("Invalid cursor: {}", e))));
            }
        }
    } else {
        None
    };

    let fetch_limit = if cursor_position.is_some() { limit * 5 } else { limit };
    let tags_for_search = req.tags.clone().filter(|t| !t.is_empty());

    // Attempt in-process embedding for vector leg.
    let (embedding_vec, vector_k) = if let Some(ref provider) = state.embed_provider {
        match provider.embed(&req.query).await {
            Ok(emb) => {
                let vec = pgvector::Vector::from(emb);
                (Some(vec), Some(60.0_f64))
            }
            Err(e) => {
                tracing::warn!(error = %e, "Embedding failed in search handler — falling back to text-only");
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // Execute hybrid search (single-model path).
    let raw_hits = match store
        .hybrid_search(
            &req.query,
            embedding_vec.as_ref(),
            fetch_limit,
            None, // created_after
            None, // created_before
            tags_for_search.as_deref(),
            Some(60.0), // bm25_k
            vector_k,
            Some(40.0), // symbolic_k
            req.source.as_deref(),
            req.audience.as_deref(),
            req.project.as_deref(),
        )
        .await
    {
        Ok(hits) => hits,
        Err(e) => {
            tracing::warn!(error = %e, "hybrid_search failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_json(&format!("Search failed: {}", e))));
        }
    };

    // Apply type_hint filter post-search.
    let raw_hits: Vec<_> = if let Some(ref th) = req.type_hint {
        raw_hits.into_iter().filter(|h| h.memory.type_hint == *th).collect()
    } else {
        raw_hits
    };

    if raw_hits.is_empty() {
        metrics::histogram!("memcp_search_results_returned").record(0.0_f64);
        let output = json!({
            "results": [],
            "next_cursor": serde_json::Value::Null,
            "has_more": false,
            "total": 0,
        });
        return (StatusCode::OK, Json(output));
    }

    // Salience re-ranking.
    let memory_ids: Vec<String> = raw_hits.iter().map(|h| h.memory.id.clone()).collect();
    let salience_data = match store.get_salience_data(&memory_ids).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "get_salience_data failed — using zero salience");
            std::collections::HashMap::new()
        }
    };

    let mut scored_hits: Vec<ScoredHit> = raw_hits
        .iter()
        .map(|h| ScoredHit {
            memory: h.memory.clone(),
            rrf_score: h.rrf_score,
            salience_score: 0.0,
            match_source: h.match_source.clone(),
            breakdown: None,
            composite_score: 0.0,
        })
        .collect();

    let salience_inputs: Vec<SalienceInput> = scored_hits
        .iter()
        .map(|h| {
            let row = salience_data.get(&h.memory.id).cloned().unwrap_or_default();
            let days_since = row
                .last_reinforced_at
                .map(|t| (Utc::now() - t).num_seconds() as f64 / 86400.0)
                .unwrap_or(365.0);
            SalienceInput {
                stability: row.stability,
                days_since_reinforced: days_since,
            }
        })
        .collect();

    let scorer = SalienceScorer::new(&state.config.salience);
    scorer.rank(&mut scored_hits, &salience_inputs);

    // Apply salience threshold filter.
    let mut scored_hits: Vec<ScoredHit> = if effective_min > 0.0 {
        scored_hits.into_iter().filter(|h| h.salience_score >= effective_min).collect()
    } else {
        scored_hits
    };

    // Compute composite score (0-1).
    if scored_hits.len() == 1 {
        scored_hits[0].composite_score = 1.0;
    } else if scored_hits.len() > 1 {
        let max_rrf = scored_hits.iter().map(|h| h.rrf_score).fold(f64::MIN, f64::max);
        let min_rrf = scored_hits.iter().map(|h| h.rrf_score).fold(f64::MAX, f64::min);
        let rrf_range = (max_rrf - min_rrf).max(1e-9);

        let max_sal = scored_hits.iter().map(|h| h.salience_score).fold(f64::MIN, f64::max);
        let min_sal = scored_hits.iter().map(|h| h.salience_score).fold(f64::MAX, f64::min);
        let sal_range = (max_sal - min_sal).max(1e-9);

        for hit in &mut scored_hits {
            let norm_rrf = (hit.rrf_score - min_rrf) / rrf_range;
            let norm_sal = (hit.salience_score - min_sal) / sal_range;
            let trust = hit.memory.trust_level as f64;
            hit.composite_score = 0.5 * norm_rrf + 0.5 * (norm_sal * trust);
        }
    }

    dedup_parent_chunks(&mut scored_hits);

    // Apply cursor-based filtering.
    let scored_hits: Vec<ScoredHit> = if let Some((last_score, ref last_id)) = cursor_position {
        scored_hits.into_iter().filter(|h| {
            let score = h.salience_score;
            if (score - last_score).abs() < f64::EPSILON {
                h.memory.id.as_str() > last_id.as_str()
            } else {
                score < last_score
            }
        }).collect()
    } else {
        scored_hits
    };

    let has_more = scored_hits.len() as i64 > limit;
    let take = if has_more { limit as usize } else { scored_hits.len() };
    let scored_hits: Vec<ScoredHit> = scored_hits.into_iter().take(take).collect();

    let next_cursor: Option<String> = if has_more {
        scored_hits.last().map(|h| encode_search_keyset_cursor(h.salience_score, &h.memory.id))
    } else {
        None
    };

    // Build results array — same shape as CLI --json output.
    let results: Vec<serde_json::Value> = scored_hits.iter().map(|h| {
        let mut entry = format_memory_json(&h.memory);
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("id".to_string(), json!(h.memory.id));
            obj.insert("salience_score".to_string(), json!(h.salience_score));
            obj.insert("composite_score".to_string(), json!((h.composite_score * 1000.0).round() / 1000.0));
            obj.insert("rrf_score".to_string(), json!(h.rrf_score));
            obj.insert("match_source".to_string(), json!(h.match_source));
        }
        apply_field_projection(entry, &req.fields)
    }).collect();

    let total = results.len();

    // Record histogram of search results returned per request.
    metrics::histogram!("memcp_search_results_returned").record(total as f64);

    let output = json!({
        "results": results,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "total": total,
    });

    (StatusCode::OK, Json(output))
}

/// Format a Memory as JSON (compact mode — same as CLI non-verbose format).
fn format_memory_json(memory: &crate::store::Memory) -> serde_json::Value {
    let mut obj = json!({
        "id": memory.id,
        "content": memory.content,
        "type_hint": memory.type_hint,
        "source": memory.source,
        "tags": memory.tags,
        "created_at": memory.created_at.to_rfc3339(),
        "actor": memory.actor,
        "actor_type": memory.actor_type,
        "audience": memory.audience,
    });
    if let Some(ref et) = memory.event_time {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("event_time".to_string(), json!(et.to_rfc3339()));
        }
    }
    if let Some(ref etp) = memory.event_time_precision {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("event_time_precision".to_string(), json!(etp));
        }
    }
    if let Some(ref ws) = memory.project {
        if let serde_json::Value::Object(ref mut map) = obj {
            map.insert("project".to_string(), json!(ws));
        }
    }
    obj
}

/// Apply field projection — only keep requested fields.
fn apply_field_projection(obj: serde_json::Value, fields: &Option<Vec<String>>) -> serde_json::Value {
    match fields {
        None => obj,
        Some(requested) if requested.is_empty() => obj,
        Some(requested) => {
            if let serde_json::Value::Object(map) = obj {
                let mut result = serde_json::Map::new();
                for field in requested {
                    if let Some(dot_pos) = field.find('.') {
                        let parent_key = &field[..dot_pos];
                        let child_key = &field[dot_pos + 1..];
                        if child_key.contains('.') { continue; }
                        if let Some(serde_json::Value::Object(nested)) = map.get(parent_key) {
                            if let Some(child_val) = nested.get(child_key) {
                                let entry = result.entry(parent_key.to_string())
                                    .or_insert_with(|| serde_json::json!({}));
                                if let serde_json::Value::Object(ref mut m) = entry {
                                    m.insert(child_key.to_string(), child_val.clone());
                                }
                            }
                        }
                    } else if let Some(val) = map.get(field.as_str()) {
                        result.insert(field.clone(), val.clone());
                    }
                }
                serde_json::Value::Object(result)
            } else {
                obj
            }
        }
    }
}
