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
use crate::search::salience::SalienceInput;
use crate::search::{SalienceScorer, ScoredHit};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{CreateMemory, ListFilter, Memory, MemoryStore};

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

/// Search memories using BM25+symbolic (no vector leg -- CLI doesn't load embedding model).
/// Results are re-ranked by salience scoring.
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
    verbose: bool,
) -> Result<()> {
    let ca = created_after.as_deref().map(parse_datetime).transpose()?;
    let cb = created_before.as_deref().map(parse_datetime).transpose()?;

    // BM25+symbolic only -- pass query_embedding=None, vector_k=None to disable vector leg
    let raw_hits = store
        .hybrid_search(
            &query,
            None,        // no query embedding
            limit,
            ca,
            cb,
            tags.as_deref(),
            Some(60.0),  // bm25_k default
            None,        // vector_k=None disables vector leg
            Some(40.0),  // symbolic_k default
            source.as_deref(),
            audience.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    if raw_hits.is_empty() {
        println!(
            "{}",
            serde_json::to_string(&json!({ "results": [], "total": 0 }))?
        );
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

    // Format results
    let results: Vec<serde_json::Value> = scored_hits
        .iter()
        .map(|h| {
            let mut entry = format_memory_json(&h.memory, verbose);
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("salience_score".to_string(), json!(h.salience_score));
                obj.insert("rrf_score".to_string(), json!(h.rrf_score));
                obj.insert("match_source".to_string(), json!(h.match_source));
            }
            entry
        })
        .collect();

    let output = json!({
        "results": results,
        "total": results.len(),
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
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

/// Show daemon status and pending work counts.
pub async fn cmd_status(store: &Arc<PostgresMemoryStore>) -> Result<()> {
    // Daemon heartbeat info
    let daemon_row = sqlx::query(
        "SELECT last_heartbeat, started_at, pid, version, worker_states \
         FROM daemon_status WHERE id = 1",
    )
    .fetch_optional(store.pool())
    .await;

    let daemon_info = match daemon_row {
        Ok(Some(row)) => {
            let heartbeat: Option<DateTime<Utc>> = row.get("last_heartbeat");
            let alive = heartbeat
                .map(|hb| (Utc::now() - hb).num_seconds() < 30)
                .unwrap_or(false);
            json!({
                "alive": alive,
                "last_heartbeat": heartbeat.map(|t| t.to_rfc3339()),
                "started_at": row.get::<Option<DateTime<Utc>>, _>("started_at").map(|t| t.to_rfc3339()),
                "pid": row.get::<Option<i32>, _>("pid"),
                "version": row.get::<Option<String>, _>("version"),
                "worker_states": row.get::<Option<serde_json::Value>, _>("worker_states"),
            })
        }
        _ => json!({ "alive": false }),
    };

    // Pending work counts
    let pending_embed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE embedding_status = 'pending'")
            .fetch_one(store.pool())
            .await
            .unwrap_or(0);

    let pending_extract: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE extraction_status = 'pending'")
            .fetch_one(store.pool())
            .await
            .unwrap_or(0);

    let total_memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
        .fetch_one(store.pool())
        .await
        .unwrap_or(0);

    let output = json!({
        "daemon": daemon_info,
        "pending": {
            "embeddings": pending_embed,
            "extractions": pending_extract,
        },
        "total_memories": total_memories,
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
