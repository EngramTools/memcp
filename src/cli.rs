/// CLI subcommand handlers for memcp
///
/// Each `cmd_*` function takes primitive args (not the Commands enum from main.rs)
/// and performs its operation against the PostgresMemoryStore directly.
///
/// Design principles:
/// - JSON output to stdout (machine-parseable for agents)
/// - Warnings/errors to stderr
/// - Short-lived: connect, execute, exit (no long-running state)
/// - Search uses BM25+symbolic only (no embedding model loaded in CLI process)

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::Row;

use crate::config::Config;
use crate::gc;
use crate::ipc::embed_via_daemon;
use crate::search::salience::SalienceInput;
use crate::search::{SalienceScorer, ScoredHit};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{
    decode_search_keyset_cursor, encode_search_keyset_cursor,
    CreateMemory, ListFilter, Memory, MemoryStore,
};

// ---------------------------------------------------------------------------
// Connection helper
// ---------------------------------------------------------------------------

/// Connect to the database and optionally run migrations.
///
/// CLI commands use `run_migrations = !skip_migrate` (migrations run by default).
pub async fn connect_store(config: &Config, skip_migrate: bool) -> Result<Arc<PostgresMemoryStore>> {
    let run_migrations = !skip_migrate;
    let store = PostgresMemoryStore::new(&config.database_url, run_migrations)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to database: {}", e))?;
    Ok(Arc::new(store))
}

// ---------------------------------------------------------------------------
// Daemon liveness check
// ---------------------------------------------------------------------------

/// Check if the daemon is alive by reading the daemon_status singleton row.
/// Returns true if last_heartbeat is within the last 30 seconds.
async fn check_daemon_alive(store: &PostgresMemoryStore) -> bool {
    let result = sqlx::query(
        "SELECT last_heartbeat FROM daemon_status WHERE id = 1"
    )
    .fetch_optional(store.pool())
    .await;

    match result {
        Ok(Some(row)) => {
            let heartbeat: Option<DateTime<Utc>> = row.get("last_heartbeat");
            match heartbeat {
                Some(hb) => {
                    let age = Utc::now() - hb;
                    age.num_seconds() < 30
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Print a stderr warning if the daemon is not running.
/// CLI-stored memories get embedding_status='pending' -- they need the daemon
/// to process embeddings and extractions asynchronously.
async fn warn_if_no_daemon(store: &PostgresMemoryStore) {
    if !check_daemon_alive(store).await {
        eprintln!(
            "warning: daemon not running -- embeddings and extractions will be pending. \
             Start with: memcp daemon"
        );
    }
}

// ---------------------------------------------------------------------------
// JSON formatting
// ---------------------------------------------------------------------------

/// Format a Memory as a JSON value with the default (compact) field set.
fn format_memory_json(memory: &Memory, verbose: bool) -> serde_json::Value {
    if verbose {
        json!({
            "id": memory.id,
            "content": memory.content,
            "type_hint": memory.type_hint,
            "source": memory.source,
            "tags": memory.tags,
            "created_at": memory.created_at.to_rfc3339(),
            "updated_at": memory.updated_at.to_rfc3339(),
            "last_accessed_at": memory.last_accessed_at.map(|t| t.to_rfc3339()),
            "access_count": memory.access_count,
            "embedding_status": memory.embedding_status,
            "extraction_status": memory.extraction_status,
            "extracted_entities": memory.extracted_entities,
            "extracted_facts": memory.extracted_facts,
            "is_consolidated_original": memory.is_consolidated_original,
            "consolidated_into": memory.consolidated_into,
            "actor": memory.actor,
            "actor_type": memory.actor_type,
            "audience": memory.audience,
        })
    } else {
        json!({
            "id": memory.id,
            "content": memory.content,
            "type_hint": memory.type_hint,
            "source": memory.source,
            "tags": memory.tags,
            "created_at": memory.created_at.to_rfc3339(),
            "actor": memory.actor,
            "actor_type": memory.actor_type,
            "audience": memory.audience,
        })
    }
}

/// Parse an ISO 8601 date string into a DateTime<Utc>.
fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    // Try full RFC3339 first, then date-only (assume start of day UTC)
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Ok(dt);
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .expect("valid time")
            .and_utc();
        return Ok(dt);
    }
    Err(anyhow::anyhow!(
        "Invalid date format: '{}'. Use ISO 8601 (e.g., 2024-01-15 or 2024-01-15T10:30:00Z)",
        s
    ))
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

/// Store a new memory. Outputs the created memory as JSON.
pub async fn cmd_store(
    store: &Arc<PostgresMemoryStore>,
    content: String,
    type_hint: String,
    source: String,
    tags: Option<Vec<String>>,
    actor: Option<String>,
    actor_type: String,
    audience: String,
) -> Result<()> {
    let input = CreateMemory {
        content,
        type_hint,
        source,
        tags,
        created_at: None,
        actor,
        actor_type,
        audience,
    };

    let memory = store
        .store(input)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Seed salience: explicit stores get stability=3.0 (stronger than auto-store's 2.5)
    if let Err(e) = store.upsert_salience(&memory.id, 3.0, 5.0, 0, None).await {
        tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience for explicit store");
    }

    println!(
        "{}",
        serde_json::to_string(&format_memory_json(&memory, false))?
    );

    warn_if_no_daemon(store).await;
    Ok(())
}

/// Search memories using the full hybrid pipeline (vector + BM25 + symbolic) when the daemon
/// is running, degrading gracefully to BM25+symbolic when the daemon is offline.
///
/// When the daemon is running, the CLI obtains an embedding vector via Unix domain socket IPC
/// (`embed_via_daemon`), enabling vector similarity search identical to MCP serve.
/// When the daemon is offline, a warning is emitted to stderr and search falls back to text-only.
///
/// Output formats:
/// - `--json` (json=true): raw JSON matching MCP serve envelope (`{ results, total }`)
/// - `--compact` (compact=true): one line per result with id, score, snippet, tags
/// - default: human-friendly list with all key fields
pub async fn cmd_search(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    query: String,
    limit: i64,
    created_after: Option<String>,
    created_before: Option<String>,
    tags: Option<Vec<String>>,
    source: Option<String>,
    audience: Option<String>,
    type_hint: Option<String>,
    verbose: bool,
    json: bool,
    compact: bool,
    cursor: Option<String>,
) -> Result<()> {
    let ca = created_after.as_deref().map(parse_datetime).transpose()?;
    let cb = created_before.as_deref().map(parse_datetime).transpose()?;

    // Decode cursor if provided to get (last_salience_score, last_id) for application-level filtering.
    let cursor_position: Option<(f64, String)> = if let Some(ref c) = cursor {
        match decode_search_keyset_cursor(c) {
            Ok(pos) => Some(pos),
            Err(e) => {
                return Err(anyhow::anyhow!("Invalid cursor: {}", e));
            }
        }
    } else {
        None
    };

    // Attempt to obtain embedding from daemon for vector leg (SCF-01).
    let query_embedding_opt = embed_via_daemon(&query).await;
    let (query_embedding_vec, vector_k) = match &query_embedding_opt {
        Some(embedding) => {
            // Full hybrid pipeline: daemon provided embedding.
            let vec = pgvector::Vector::from(embedding.clone());
            (Some(vec), Some(60.0_f64))
        }
        None => {
            // Daemon offline — degrade gracefully to text-only search.
            eprintln!("warning: daemon offline — falling back to text-only search (results may be degraded). Start with: memcp daemon");
            (None, None)
        }
    };

    // Build tags filter. type_hint is applied post-search as a result filter
    // (hybrid_search doesn't expose a type_hint column filter; post-filter is simple and correct).
    let tags_for_search = tags.clone().filter(|t| !t.is_empty());

    // Fetch a larger candidate pool when using cursor pagination (need candidates beyond cursor pos).
    let fetch_limit = if cursor_position.is_some() { limit * 5 } else { limit };

    let raw_hits = store
        .hybrid_search(
            &query,
            query_embedding_vec.as_ref(),
            fetch_limit,
            ca,
            cb,
            tags_for_search.as_deref(),
            Some(60.0),  // bm25_k default
            vector_k,    // Some(60.0) when daemon alive, None when offline
            Some(40.0),  // symbolic_k default
            source.as_deref(),
            audience.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Apply type_hint filter post-search (symbolic leg doesn't filter by type_hint column).
    let raw_hits: Vec<_> = if let Some(ref th) = type_hint {
        raw_hits.into_iter().filter(|h| h.memory.type_hint == *th).collect()
    } else {
        raw_hits
    };

    if raw_hits.is_empty() {
        let output = json!({
            "results": [],
            "next_cursor": null,
            "has_more": false,
            "total": 0,
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    // Convert to ScoredHit for salience re-ranking
    let memory_ids: Vec<String> = raw_hits.iter().map(|h| h.memory.id.clone()).collect();
    let salience_data = store
        .get_salience_data(&memory_ids)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut scored_hits: Vec<ScoredHit> = raw_hits
        .iter()
        .map(|h| ScoredHit {
            memory: h.memory.clone(),
            rrf_score: h.rrf_score,
            salience_score: 0.0,
            match_source: h.match_source.clone(),
            breakdown: None,
        })
        .collect();

    let salience_inputs: Vec<SalienceInput> = scored_hits
        .iter()
        .map(|h| {
            let row = salience_data
                .get(&h.memory.id)
                .cloned()
                .unwrap_or_default();
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

    let scorer = SalienceScorer::new(&config.salience);
    scorer.rank(&mut scored_hits, &salience_inputs);

    // Apply cursor-based filtering: skip items at or before the cursor position.
    // Cursor encodes (salience_score, id) of the LAST item on the previous page.
    // Skip items where: score > last_score OR (score == last_score AND id <= last_id).
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

    // Take limit items, detect if more remain.
    let has_more = scored_hits.len() as i64 > limit;
    let take = if has_more { limit as usize } else { scored_hits.len() };
    let scored_hits: Vec<ScoredHit> = scored_hits.into_iter().take(take).collect();

    // Build next_cursor from the last item's (salience_score, id) — keyset cursor.
    let next_cursor: Option<String> = if has_more {
        scored_hits.last().map(|h| encode_search_keyset_cursor(h.salience_score, &h.memory.id))
    } else {
        None
    };

    // Format results according to output mode.
    if json {
        // --json: MCP-compatible JSON envelope. id always present at top level (SCF-03).
        let results: Vec<serde_json::Value> = scored_hits
            .iter()
            .map(|h| {
                let mut entry = format_memory_json(&h.memory, verbose || true);
                if let Some(obj) = entry.as_object_mut() {
                    // Ensure id is always top-level (SCF-03)
                    obj.insert("id".to_string(), json!(h.memory.id));
                    obj.insert("salience_score".to_string(), json!(h.salience_score));
                    obj.insert("rrf_score".to_string(), json!(h.rrf_score));
                    obj.insert("match_source".to_string(), json!(h.match_source));
                }
                entry
            })
            .collect();

        let output = json!({
            "results": results,
            "next_cursor": next_cursor,
            "has_more": has_more,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else if compact {
        // --compact: one line per result: "{id_short} {score:.3} {snippet_80} [{tags}]"
        for h in &scored_hits {
            let id_short = &h.memory.id[..8.min(h.memory.id.len())];
            let snippet: String = h.memory.content
                .chars()
                .take(80)
                .collect();
            let snippet = if h.memory.content.len() > 80 {
                format!("{}…", snippet)
            } else {
                snippet
            };
            let tags_str = h.memory.tags
                .as_ref()
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            println!(
                "{} {:.3} {} [{}]",
                id_short, h.salience_score, snippet, tags_str
            );
        }
        // Print next cursor for compact mode if has_more
        if has_more {
            if let Some(ref c) = next_cursor {
                println!("Next: {}", c);
            }
        }
    } else {
        // Default: human-friendly JSON list with id, content, tags, score, type_hint, created_at.
        // id always present at top level (SCF-03).
        let results: Vec<serde_json::Value> = scored_hits
            .iter()
            .map(|h| {
                let mut entry = format_memory_json(&h.memory, verbose);
                if let Some(obj) = entry.as_object_mut() {
                    // Ensure id is always top-level (SCF-03)
                    obj.insert("id".to_string(), json!(h.memory.id));
                    obj.insert("salience_score".to_string(), json!(h.salience_score));
                    obj.insert("rrf_score".to_string(), json!(h.rrf_score));
                    obj.insert("match_source".to_string(), json!(h.match_source));
                }
                entry
            })
            .collect();

        let output = json!({
            "results": results,
            "next_cursor": next_cursor,
            "has_more": has_more,
            "total": results.len(),
        });
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}

/// Show recent memories for session handoff.
///
/// Uses `list` with a time filter derived from the `--since` duration string.
/// Supports "30m", "1h", "2h", "1d" etc.
pub async fn cmd_recent(
    store: &Arc<PostgresMemoryStore>,
    since: String,
    source: Option<String>,
    actor: Option<String>,
    limit: i64,
    verbose: bool,
) -> Result<()> {
    let duration = parse_duration(&since)?;
    let created_after = Utc::now() - duration;

    let filter = ListFilter {
        type_hint: None,
        source,
        created_after: Some(created_after),
        created_before: None,
        updated_after: None,
        updated_before: None,
        limit,
        cursor: None,
        actor,
        audience: None,
    };

    let result = store
        .list(filter)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let memories: Vec<serde_json::Value> = result
        .memories
        .iter()
        .map(|m| format_memory_json(m, verbose))
        .collect();

    let output = json!({
        "memories": memories,
        "count": memories.len(),
        "since": since,
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Parse a human-readable duration string like "30m", "1h", "2h", "1d".
fn parse_duration(s: &str) -> Result<chrono::Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow::anyhow!("Empty duration string"));
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().map_err(|_| {
        anyhow::anyhow!("Invalid duration '{}'. Use format like '30m', '1h', '2h', '1d'", s)
    })?;

    match unit {
        "m" => Ok(chrono::Duration::minutes(num)),
        "h" => Ok(chrono::Duration::hours(num)),
        "d" => Ok(chrono::Duration::days(num)),
        _ => Err(anyhow::anyhow!(
            "Unknown duration unit '{}'. Use 'm' (minutes), 'h' (hours), or 'd' (days)", unit
        )),
    }
}

/// List memories with optional filters and cursor-based pagination.
pub async fn cmd_list(
    store: &Arc<PostgresMemoryStore>,
    type_hint: Option<String>,
    source: Option<String>,
    created_after: Option<String>,
    created_before: Option<String>,
    updated_after: Option<String>,
    updated_before: Option<String>,
    limit: i64,
    cursor: Option<String>,
    actor: Option<String>,
    audience: Option<String>,
    verbose: bool,
) -> Result<()> {
    let filter = ListFilter {
        type_hint,
        source,
        created_after: created_after.as_deref().map(parse_datetime).transpose()?,
        created_before: created_before.as_deref().map(parse_datetime).transpose()?,
        updated_after: updated_after.as_deref().map(parse_datetime).transpose()?,
        updated_before: updated_before.as_deref().map(parse_datetime).transpose()?,
        limit,
        cursor,
        actor,
        audience,
    };

    let result = store
        .list(filter)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let memories: Vec<serde_json::Value> = result
        .memories
        .iter()
        .map(|m| format_memory_json(m, verbose))
        .collect();

    let output = json!({
        "memories": memories,
        "count": memories.len(),
        "next_cursor": result.next_cursor,
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Retrieve a single memory by ID.
pub async fn cmd_get(store: &Arc<PostgresMemoryStore>, id: &str) -> Result<()> {
    let memory = store.get(id).await.map_err(|e| anyhow::anyhow!("{}", e))?;
    println!(
        "{}",
        serde_json::to_string(&format_memory_json(&memory, true))?
    );
    Ok(())
}

/// Delete a memory by ID (permanent).
pub async fn cmd_delete(store: &Arc<PostgresMemoryStore>, id: &str) -> Result<()> {
    store.delete(id).await.map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("{}", serde_json::to_string(&json!({ "deleted": id }))?);
    Ok(())
}

/// Reinforce a memory to boost its salience.
pub async fn cmd_reinforce(
    store: &Arc<PostgresMemoryStore>,
    id: &str,
    rating: &str,
) -> Result<()> {
    let row = store
        .reinforce_salience(id, rating)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let output = json!({
        "memory_id": id,
        "rating": rating,
        "stability": row.stability,
        "difficulty": row.difficulty,
        "reinforcement_count": row.reinforcement_count,
        "last_reinforced_at": row.last_reinforced_at.map(|t| t.to_rfc3339()),
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

/// Format a timestamp as a human-readable relative time string (e.g., "5m ago").
fn format_relative_time(dt: DateTime<Utc>) -> String {
    let secs = (Utc::now() - dt).num_seconds().max(0);
    match secs {
        s if s < 60 => format!("{}s ago", s),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

/// Build the status JSON value (extracted for testability).
pub async fn build_status(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    check: bool,
) -> Result<(serde_json::Value, bool, Option<DateTime<Utc>>, i32, i32)> {
    // Daemon heartbeat info + sidecar fields + GC metrics
    let daemon_row = sqlx::query(
        "SELECT last_heartbeat, started_at, pid, version, worker_states, \
                last_ingest_at, ingest_count_today, watched_file_count, \
                embedding_model, embedding_dimension, \
                last_gc_at, gc_pruned_total, gc_dedup_merges, filter_stats \
         FROM daemon_status WHERE id = 1",
    )
    .fetch_optional(store.pool())
    .await;

    let (alive, daemon_info, last_ingest_at, ingest_count_today, watched_file_count,
         embedding_model, embedding_dimension, gc_info) = match daemon_row {
        Ok(Some(row)) => {
            let heartbeat: Option<DateTime<Utc>> = row.get("last_heartbeat");
            let alive = heartbeat
                .map(|hb| (Utc::now() - hb).num_seconds() < 30)
                .unwrap_or(false);
            let last_ingest: Option<DateTime<Utc>> = row.get("last_ingest_at");
            let ingest_today: Option<i32> = row.get("ingest_count_today");
            let watched: Option<i32> = row.get("watched_file_count");
            let model: Option<String> = row.get("embedding_model");
            let dimension: Option<i32> = row.get("embedding_dimension");

            let last_gc_at: Option<DateTime<Utc>> = row.get("last_gc_at");
            let gc_pruned_total: Option<i32> = row.get("gc_pruned_total");
            let gc_dedup_merges: Option<i32> = row.get("gc_dedup_merges");
            let filter_stats: Option<serde_json::Value> = row.get("filter_stats");

            let gc = json!({
                "last_run_at": last_gc_at.map(|t| t.to_rfc3339()),
                "pruned_total": gc_pruned_total.unwrap_or(0),
                "dedup_merges": gc_dedup_merges.unwrap_or(0),
                "filter_stats": filter_stats.unwrap_or_else(|| json!({})),
            });

            let info = json!({
                "alive": alive,
                "last_heartbeat": heartbeat.map(|t| t.to_rfc3339()),
                "started_at": row.get::<Option<DateTime<Utc>>, _>("started_at").map(|t| t.to_rfc3339()),
                "pid": row.get::<Option<i32>, _>("pid"),
                "version": row.get::<Option<String>, _>("version"),
                "worker_states": row.get::<Option<serde_json::Value>, _>("worker_states"),
            });
            (alive, info, last_ingest, ingest_today.unwrap_or(0), watched.unwrap_or(0),
             model, dimension, gc)
        }
        _ => (false, json!({"alive": false}), None, 0, 0, None, None,
              json!({ "last_run_at": null, "pruned_total": 0, "dedup_merges": 0, "filter_stats": {} })),
    };

    // Pending work counts (exclude soft-deleted)
    let pending_embed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE embedding_status = 'pending' AND deleted_at IS NULL")
            .fetch_one(store.pool())
            .await
            .unwrap_or(0);

    let pending_extract: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE extraction_status = 'pending' AND deleted_at IS NULL")
            .fetch_one(store.pool())
            .await
            .unwrap_or(0);

    let total_memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL")
        .fetch_one(store.pool())
        .await
        .unwrap_or(0);

    // Deep health check (when --check is passed)
    let checks = if check {
        // 1. Database reachable (already connected, but verify with a ping)
        let db_ok = sqlx::query("SELECT 1").fetch_one(store.pool()).await.is_ok();

        // 2. Ollama responding (if summarization is configured)
        let ollama_ok = if config.summarization.enabled {
            let url = format!("{}/api/version", config.summarization.ollama_base_url);
            reqwest::get(&url).await.map(|r| r.status().is_success()).unwrap_or(false)
        } else {
            true // not configured = not a failure
        };

        // 3. Model cache present on disk
        let cache_dir = dirs::cache_dir()
            .unwrap_or_default()
            .join("fastembed_cache");
        let model_cache_ok = cache_dir.exists()
            && std::fs::read_dir(&cache_dir).map(|mut d| d.next().is_some()).unwrap_or(false);

        // 4. Watch paths exist
        let watch_paths_ok = if config.auto_store.enabled {
            config.auto_store.watch_paths.iter().all(|p| {
                let expanded = crate::auto_store::watcher::expand_tilde(p);
                expanded.exists() || expanded.parent().map(|par| par.exists()).unwrap_or(false)
            })
        } else {
            true
        };

        Some(json!({
            "database": db_ok,
            "ollama": ollama_ok,
            "model_cache": model_cache_ok,
            "watch_paths": watch_paths_ok,
        }))
    } else {
        None
    };

    // Build full JSON output
    let mut output = json!({
        "daemon": daemon_info,
        "pending": { "embeddings": pending_embed, "extractions": pending_extract },
        "total_memories": total_memories,
        "sidecar": {
            "last_ingest_at": last_ingest_at.map(|t| t.to_rfc3339()),
            "ingest_count_today": ingest_count_today,
            "watched_file_count": watched_file_count,
        },
        "model": {
            "name": embedding_model,
            "dimension": embedding_dimension,
        },
        "status_line": {
            "format": config.status_line.format,
        },
        "gc": gc_info,
    });
    if let Some(checks) = checks {
        output.as_object_mut().unwrap().insert("checks".to_string(), checks);
    }

    Ok((output, alive, last_ingest_at, pending_embed as i32 + pending_extract as i32, total_memories as i32))
}

/// Show daemon status and pending work counts.
pub async fn cmd_status(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    pretty: bool,
    check: bool,
) -> Result<()> {
    let (output, alive, last_ingest_at, pending_total, total_memories) =
        build_status(store, config, check).await?;

    if pretty {
        let icon = if alive { "\u{2705}" } else { "\u{274c}" };
        if alive {
            // Uptime
            let uptime_str = output.get("daemon")
                .and_then(|d| d.get("started_at"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .map(|t| format_relative_time(t))
                .unwrap_or_else(|| "?".to_string());

            // Pending with backlog warning
            let pending_str = if pending_total > 50 {
                format!("\u{26a0} {} pending", pending_total)
            } else {
                format!("{} pending", pending_total)
            };

            // Last ingest
            let ingest_str = last_ingest_at
                .map(|t| format!("last ingest {}", format_relative_time(t)))
                .unwrap_or_else(|| "no ingests yet".to_string());

            println!("{} daemon up {} | {} | {} | {} memories",
                icon, uptime_str, pending_str, ingest_str, total_memories);
        } else {
            println!("{} daemon down", icon);
        }

        if let Some(checks) = output.get("checks") {
            let check_line: Vec<String> = ["database", "ollama", "model_cache", "watch_paths"]
                .iter()
                .map(|k| {
                    let ok = checks.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
                    let ci = if ok { "\u{2705}" } else { "\u{274c}" };
                    format!("{}: {}", k.replace('_', " "), ci)
                })
                .collect();
            println!("  {}", check_line.join("  "));
        }
    } else {
        // JSON output
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Feedback command
// ---------------------------------------------------------------------------

/// Provide explicit relevance feedback for a memory.
///
/// "useful" increases FSRS stability (multiplier 1.5) — the memory was helpful.
/// "irrelevant" decreases FSRS stability sharply (multiplier 0.2) — the memory was noise.
///
/// Fire-and-forget: outputs `{"ok": true, "id": "...", "signal": "..."}` on success.
/// Error handling is done by the caller in main.rs.
pub async fn cmd_feedback(
    store: &Arc<PostgresMemoryStore>,
    id: &str,
    signal: &str,
) -> Result<()> {
    store
        .apply_feedback(id, signal)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!(
        "{}",
        serde_json::to_string(&json!({ "ok": true, "id": id, "signal": signal }))?
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// GC command
// ---------------------------------------------------------------------------

/// Run or preview garbage collection.
///
/// With `--dry-run`: prints the list of candidates that would be pruned (no changes made).
/// Without `--dry-run`: executes GC and prints a summary of pruned/expired/hard-purged counts.
pub async fn cmd_gc(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    dry_run: bool,
    salience_threshold_override: Option<f64>,
    min_age_days_override: Option<u32>,
) -> Result<()> {
    // Apply any flag overrides to the config
    let mut gc_config = config.gc.clone();
    if let Some(t) = salience_threshold_override {
        gc_config.salience_threshold = t;
    }
    if let Some(d) = min_age_days_override {
        gc_config.min_age_days = d;
    }

    let result = gc::run_gc(store, &gc_config, dry_run)
        .await
        .map_err(|e| anyhow::anyhow!("GC failed: {}", e))?;

    if let Some(reason) = &result.skipped_reason {
        let output = serde_json::json!({
            "status": "skipped",
            "reason": reason,
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    if dry_run {
        // Show up to 20 candidates
        let show_count = result.candidates.len().min(20);
        let shown: Vec<&gc::GcCandidate> = result.candidates.iter().take(show_count).collect();
        let truncated = result.candidates.len() > 20;

        let output = serde_json::json!({
            "status": "dry_run",
            "total_candidates": result.pruned_count,
            "total_expired": result.expired_count,
            "showing": show_count,
            "truncated": truncated,
            "candidates": shown,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        let output = serde_json::json!({
            "status": "ok",
            "pruned": result.pruned_count,
            "expired": result.expired_count,
            "hard_purged": result.hard_purged_count,
        });
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Statusline install / remove
// ---------------------------------------------------------------------------

/// Install the Claude Code status line script to ~/.claude/scripts/.
pub fn cmd_statusline_install() -> Result<()> {
    let claude_scripts = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".claude")
        .join("scripts");

    std::fs::create_dir_all(&claude_scripts)?;

    let dest = claude_scripts.join("memcp-statusline.sh");
    let script = include_str!("../scripts/memcp-statusline.sh");
    std::fs::write(&dest, script)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }

    println!("Installed: {}", dest.display());
    println!();
    println!("Add to ~/.claude/settings.json:");
    println!();
    println!("  {{");
    println!("    \"statusLine\": {{");
    println!("      \"type\": \"command\",");
    println!("      \"command\": \"{}\"", dest.display());
    println!("    }}");
    println!("  }}");
    println!();
    println!("Configure format in memcp.toml (optional):");
    println!("  [status_line]");
    println!("  format = \"ingest\"  # or \"pending\" or \"state\"");
    Ok(())
}

/// Remove the Claude Code status line script from ~/.claude/scripts/.
pub fn cmd_statusline_remove() -> Result<()> {
    let dest = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?
        .join(".claude")
        .join("scripts")
        .join("memcp-statusline.sh");

    if dest.exists() {
        std::fs::remove_file(&dest)?;
        println!("Removed: {}", dest.display());
        println!("Don't forget to remove the statusLine block from ~/.claude/settings.json");
    } else {
        println!("Not installed: {}", dest.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_relative_time() {
        let now = Utc::now();
        assert!(format_relative_time(now).contains("s ago"));
        assert!(format_relative_time(now - chrono::Duration::minutes(5)).contains("5m ago"));
        assert!(format_relative_time(now - chrono::Duration::hours(2)).contains("2h ago"));
        assert!(format_relative_time(now - chrono::Duration::days(3)).contains("3d ago"));
    }

    #[test]
    fn test_format_relative_time_negative_clamps_to_zero() {
        // Future time should clamp to 0s ago
        let future = Utc::now() + chrono::Duration::hours(1);
        assert!(format_relative_time(future).contains("0s ago"));
    }
}
