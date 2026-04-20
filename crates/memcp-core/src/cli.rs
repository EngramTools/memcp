//! CLI subcommand handlers.
//!
//! Each cmd_* function implements a CLI subcommand (store, search, list, get, delete, etc.).
//! Called from the binary crate's main.rs dispatcher. Uses storage/ for DB access,
//! intelligence/ for search/recall, and transport/ipc for daemon communication.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::Row;

use crate::config::Config;
use crate::gc;
use crate::ipc::{embed_multi_via_daemon, embed_via_daemon, rerank_via_daemon};
use crate::pipeline::temporal::extract_event_time;
use crate::search::salience::SalienceInput;
use crate::search::{SalienceScorer, ScoredHit};
use crate::store::postgres::PostgresMemoryStore;
use crate::store::{
    decode_search_keyset_cursor, encode_search_keyset_cursor, CreateMemory, ListFilter, Memory,
    MemoryStore, UpdateMemory,
};

// ---------------------------------------------------------------------------
// Connection helper
// ---------------------------------------------------------------------------

/// Connect to the database and optionally run migrations.
///
/// CLI commands use `run_migrations = !skip_migrate` (migrations run by default).
pub async fn connect_store(
    config: &Config,
    skip_migrate: bool,
) -> Result<Arc<PostgresMemoryStore>> {
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
    let result = sqlx::query("SELECT last_heartbeat FROM daemon_status WHERE id = 1")
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
            "event_time": memory.event_time.map(|t| t.to_rfc3339()),
            "event_time_precision": memory.event_time_precision,
            "project": memory.project,
            "trust_level": memory.trust_level,
            "session_id": memory.session_id,
            "agent_role": memory.agent_role,
        })
    } else {
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
        // Include new fields only when non-null (saves tokens for agents)
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
        // Include provenance fields only when non-default/non-null (saves tokens for agents)
        if memory.trust_level != 0.0 {
            if let serde_json::Value::Object(ref mut map) = obj {
                map.insert("trust_level".to_string(), json!(memory.trust_level));
            }
        }
        if let Some(ref sid) = memory.session_id {
            if let serde_json::Value::Object(ref mut map) = obj {
                map.insert("session_id".to_string(), json!(sid));
            }
        }
        if let Some(ref ar) = memory.agent_role {
            if let serde_json::Value::Object(ref mut map) = obj {
                map.insert("agent_role".to_string(), json!(ar));
            }
        }
        obj
    }
}

/// Truncate content to at most `max_chars` Unicode scalar values.
///
/// Returns `(truncated_content, was_truncated)`.
/// When truncated, appends "..." so the agent knows to `memcp get <id>` for full content.
fn truncate_content(content: &str, max_chars: usize) -> (String, bool) {
    if content.chars().count() <= max_chars {
        (content.to_string(), false)
    } else {
        let truncated: String = content.chars().take(max_chars).collect();
        (format!("{}...", truncated), true)
    }
}

/// Build a ready-made `memcp search --tags ...` command from a slice of shared tags.
///
/// Returns an empty string when no tags are provided (caller skips the hint field).
fn build_related_hint(shared_tags: &[String]) -> String {
    if shared_tags.is_empty() {
        return String::new();
    }
    format!("memcp search --tags {}", shared_tags.join(","))
}

/// Parse an ISO 8601 date string into a DateTime<Utc>.
fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    // Try full RFC3339 first, then date-only (assume start of day UTC)
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Ok(dt);
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date.and_hms_opt(0, 0, 0).expect("valid time").and_utc();
        return Ok(dt);
    }
    Err(anyhow::anyhow!(
        "Invalid date format: '{}'. Use ISO 8601 (e.g., 2024-01-15 or 2024-01-15T10:30:00Z)",
        s
    ))
}

/// Resolve project with CLI flag > MEMCP_PROJECT env var (with MEMCP_WORKSPACE fallback) > config default_project precedence.
///
/// Returns the first non-empty project from the chain, or None if all are absent/empty.
pub fn resolve_project(cli_flag: Option<String>, config: &Config) -> Option<String> {
    cli_flag
        .or_else(|| {
            std::env::var("MEMCP_PROJECT")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("MEMCP_WORKSPACE")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .or_else(|| config.project.default_project.clone())
}

/// Resolve content from a positional arg or --stdin flag.
///
/// Returns Err if both are provided. Returns Ok(None) if neither is provided.
/// Callers decide whether None is valid (update: yes; store: no).
pub fn resolve_content_arg(content: Option<String>, stdin: bool) -> Result<Option<String>> {
    match (content, stdin) {
        (Some(_), true) => Err(anyhow::anyhow!("Cannot specify both content and --stdin")),
        (Some(c), false) => Ok(Some(c)),
        (None, true) => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(Some(buf.trim_end().to_string()))
        }
        (None, false) => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Remote dispatch
// ---------------------------------------------------------------------------

/// Send a command to a remote memcp daemon via HTTP API.
///
/// Routes CLI commands through the HTTP API when `--remote <url>` or `MEMCP_URL` is set.
/// Returns the parsed JSON response. Returns Err on non-success HTTP status or network error.
pub async fn dispatch_remote(
    base_url: &str,
    command: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let url = format!("{}/v1/{}", base_url.trim_end_matches('/'), command);
    let resp = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Remote request to {} failed: {}", url, e))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read remote response: {}", e))?;

    let json: serde_json::Value = serde_json::from_str(&body_text)
        .unwrap_or_else(|_| serde_json::json!({"error": body_text}));

    if !status.is_success() {
        let msg = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("remote error");
        anyhow::bail!("Remote memcp error ({}): {}", status.as_u16(), msg);
    }

    Ok(json)
}

/// Send a GET request to a remote memcp daemon and return the raw response body.
///
/// Used by `memcp export --remote` to fetch exported data from a hosted instance.
/// Unlike `dispatch_remote` which expects JSON, this returns raw bytes as a String
/// (the export data in the requested format: jsonl, csv, or markdown).
pub async fn dispatch_remote_get(
    base_url: &str,
    command: &str,
    params: &[(&str, String)],
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let url = format!("{}/v1/{}", base_url.trim_end_matches('/'), command);

    let mut req = client.get(&url);
    for (key, value) in params {
        req = req.query(&[(key, value)]);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Remote GET request to {} failed: {}", url, e))?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read remote response: {}", e))?;

    if !status.is_success() {
        // Try to parse an error message from JSON.
        let msg = serde_json::from_str::<serde_json::Value>(&body_text)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| body_text.clone());
        anyhow::bail!("Remote memcp error ({}): {}", status.as_u16(), msg);
    }

    Ok(body_text)
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

/// Store a new memory. Outputs the created memory as JSON.
///
/// Enforces max_memories resource cap if configured. Exits with error when cap exceeded.
/// Content is required — provide as positional arg or via --stdin (resolved by caller).
#[allow(clippy::too_many_arguments)] // CLI handler args map to command-line flags; struct refactor deferred
pub async fn cmd_store(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    content: Option<String>,
    type_hint: String,
    source: String,
    tags: Option<Vec<String>>,
    actor: Option<String>,
    actor_type: String,
    audience: String,
    idempotency_key: Option<String>,
    wait: bool,
    project: Option<String>,
    trust_level: Option<f32>,
    session_id: Option<String>,
    agent_role: Option<String>,
    no_redact: bool,
    knowledge_tier: Option<String>,
    source_ids: Option<Vec<String>>,
) -> Result<()> {
    let content = content.ok_or_else(|| {
        anyhow::anyhow!("Content is required — provide as argument or use --stdin")
    })?;

    // Validate input sizes
    crate::validation::validate_content(&content, &config.input_limits)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    if let Some(ref t) = tags {
        crate::validation::validate_tags(t, &config.input_limits)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }

    // Redact secrets/PII unless --no-redact is set
    let content =
        if !no_redact && (config.redaction.secrets_enabled || config.redaction.pii_enabled) {
            match crate::pipeline::redaction::RedactionEngine::from_config(&config.redaction) {
                Ok(engine) => match engine.redact(&content) {
                    Ok(result) => {
                        if result.was_redacted {
                            eprintln!(
                                "Warning: {} items redacted (categories: {})",
                                result.redaction_count,
                                result.categories.join(", ")
                            );
                        }
                        result.content
                    }
                    Err(e) => {
                        eprintln!("Error: redaction failed — {}", e);
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("Error: failed to initialize redaction engine — {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            content
        };

    // Resource cap: max_memories — hard reject at hard_cap_percent
    if let Some(max) = config.resource_caps.max_memories {
        let count = store
            .count_live_memories()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check memory count: {}", e))?;
        let ratio = count as f64 / max as f64;
        let hard_cap = config.resource_limits.hard_cap_percent as f64 / 100.0;
        if ratio >= hard_cap {
            eprintln!("Error: Resource cap exceeded — max_memories limit is {} (current: {}, hard_cap: {}%)", max, count, config.resource_limits.hard_cap_percent);
            std::process::exit(1);
        }
        // Capacity warning
        let warn_threshold = config.resource_limits.warn_percent as f64 / 100.0;
        if ratio >= warn_threshold {
            eprintln!(
                "Warning: Memory usage at {}%. Upgrade storage at engram.host/upgrade",
                (ratio * 100.0).round()
            );
        }
    }

    // Extract temporal event time from content using regex patterns.
    let temporal_result = extract_event_time(&content, config.user.birth_year, Utc::now());
    let (event_time, event_time_precision) = match temporal_result {
        Some((dt, precision)) => (Some(dt), Some(precision.as_str().to_string())),
        None => (None, None),
    };

    let input = CreateMemory {
        content,
        type_hint,
        source,
        tags,
        created_at: None,
        actor,
        actor_type,
        audience,
        idempotency_key,
        event_time,
        event_time_precision,
        project,
        trust_level,
        session_id,
        agent_role,
        write_path: Some("explicit_store".to_string()),
        knowledge_tier,
        source_ids,
        reply_to_id: None,
    };

    let memory = store
        .store(input)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Seed salience: explicit stores get stability=3.0 (stronger than auto-store's 2.5)
    if let Err(e) = store.upsert_salience(&memory.id, 3.0, 5.0, 0, None).await {
        tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience for explicit store");
    }

    // Sync store: poll embedding_status until complete (or timeout)
    let mut response_json = format_memory_json(&memory, false);
    if wait {
        let timeout = std::time::Duration::from_secs(config.store.sync_timeout_secs);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= timeout {
                // Timeout — embedding still pending
                if let serde_json::Value::Object(ref mut map) = response_json {
                    map.insert("embedding_status".to_string(), serde_json::json!("pending"));
                }
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match store.get(&memory.id).await {
                Ok(m) => {
                    if m.embedding_status == "complete" || m.embedding_status == "failed" {
                        if let serde_json::Value::Object(ref mut map) = response_json {
                            map.insert(
                                "embedding_status".to_string(),
                                serde_json::json!(m.embedding_status),
                            );
                        }
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    println!("{}", serde_json::to_string(&response_json)?);

    warn_if_no_daemon(store).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_ingest — Phase 24.5 Plan 04 batch ingestion CLI subcommand
// ---------------------------------------------------------------------------

/// CLI entry for `memcp ingest`. Resolves input from `--file`, `--message`, or
/// stdin (explicit `--stdin` flag or piped) and dispatches to
/// `cmd_ingest_from_reader` which is testable without touching real stdin.
///
/// Emits pretty-printed `{results, summary}` JSON to stdout.
#[allow(clippy::too_many_arguments)] // CLI args map to command-line flags
pub async fn cmd_ingest(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    file: Option<String>,
    message: Option<String>,
    stdin_flag: bool,
    source: String,
    session_id: String,
    project: String,
) -> Result<()> {
    use std::io::IsTerminal;

    // Dispatch by input mode. The `--message` path wraps the JSON object into a
    // single-element batch to avoid re-using the JSONL/array auto-detect path.
    let value = if let Some(msg_str) = message {
        cmd_ingest_from_message_str(store, config, &msg_str, source, session_id, project).await?
    } else if let Some(path) = file {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read --file '{}': {}", path, e))?;
        cmd_ingest_from_reader(
            store,
            config,
            std::io::Cursor::new(raw),
            source,
            session_id,
            project,
        )
        .await?
    } else if stdin_flag || !std::io::stdin().is_terminal() {
        cmd_ingest_from_reader(
            store,
            config,
            std::io::stdin(),
            source,
            session_id,
            project,
        )
        .await?
    } else {
        anyhow::bail!("no input: provide --file, --message, or pipe via stdin");
    };

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Testable seam for `cmd_ingest`: reads an arbitrary `Read`, auto-detects
/// JSONL vs JSON array via `parse_ingest_stream`, and dispatches to the shared
/// batch loop (same code path as HTTP / MCP).
pub async fn cmd_ingest_from_reader<R: std::io::Read>(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    mut reader: R,
    source: String,
    session_id: String,
    project: String,
) -> Result<serde_json::Value> {
    let mut raw = String::new();
    reader
        .read_to_string(&mut raw)
        .map_err(|e| anyhow::anyhow!("failed to read ingest input: {}", e))?;
    let messages = crate::transport::api::ingest::parse_ingest_stream(&raw)
        .map_err(|e| anyhow::anyhow!("failed to parse ingest input: {}", e))?;

    let req = crate::transport::api::types::IngestRequest {
        messages,
        source,
        session_id,
        project,
    };
    run_cmd_ingest(store, config, req).await
}

/// Internal helper for `--message '{...}'` — deserializes a single JSON object
/// into an `IngestMessage` and wraps it in a 1-element batch.
async fn cmd_ingest_from_message_str(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    msg_str: &str,
    source: String,
    session_id: String,
    project: String,
) -> Result<serde_json::Value> {
    let msg: crate::transport::api::types::IngestMessage = serde_json::from_str(msg_str)
        .map_err(|e| anyhow::anyhow!("invalid --message JSON: {}", e))?;
    let req = crate::transport::api::types::IngestRequest {
        messages: vec![msg],
        source,
        session_id,
        project,
    };
    run_cmd_ingest(store, config, req).await
}

/// Build a `ProcessMessageContext` from CLI resources and invoke
/// `run_ingest_batch_with_ctx`. CLI is DB-direct like other subcommands — no
/// AppState, no rate-limit layer. Skips summarization / content filter /
/// embedding enqueue (each is optional in the shared helper).
async fn run_cmd_ingest(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    req: crate::transport::api::types::IngestRequest,
) -> Result<serde_json::Value> {
    let helper_ctx = crate::pipeline::auto_store::shared::ProcessMessageContext {
        store,
        redaction_engine: None,
        content_filter: None,
        summarization_provider: None,
        embed_sender: None,
        extract_sender: None,
    };

    let (_status, value) = crate::transport::api::ingest::run_ingest_batch_with_ctx(
        &helper_ctx,
        config.ingest.max_batch_size,
        config.ingest.max_content_size,
        config.user.birth_year,
        req,
    )
    .await;
    Ok(value)
}

/// Update a memory's content or metadata in place.
///
/// At least one of content, type_hint, source, or tags must be provided.
/// If content changes, embedding_status is reset to "pending" so the daemon re-embeds.
/// Note: Content filtering is skipped here — consistent with cmd_store. Content filtering
/// is a server-layer concern only (MCP serve mode).
#[allow(clippy::too_many_arguments)] // CLI handler args map to command-line flags; struct refactor deferred
pub async fn cmd_update(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    id: String,
    content: Option<String>,
    type_hint: Option<String>,
    source: Option<String>,
    tags: Option<Vec<String>>,
    wait: bool,
) -> Result<()> {
    // Validate: at least one field must be provided
    if content.is_none() && type_hint.is_none() && source.is_none() && tags.is_none() {
        return Err(anyhow::anyhow!(
            "At least one field required: content, --type-hint, --source, or --tags"
        ));
    }

    let has_content_change = content.is_some();

    let input = UpdateMemory {
        content,
        type_hint,
        source,
        tags,
        trust_level: None,
    };

    let memory = store
        .update(&id, input)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // If content changed, reset embedding_status to "pending" so daemon re-embeds.
    // store.update() only updates fields in UpdateMemory — it does NOT reset
    // embedding_status. CLI has no pipeline access, so daemon polling handles re-embed.
    if has_content_change {
        if let Err(e) = store.update_embedding_status(&id, "pending").await {
            tracing::warn!("Failed to reset embedding_status: {}", e);
            // Fail-open: don't abort the update
        }
    }

    // Format output (matches cmd_store output style)
    let mut response_json = format_memory_json(&memory, false);

    // --wait: poll embedding_status until complete (reuse cmd_store pattern)
    if wait && has_content_change {
        let timeout = std::time::Duration::from_secs(config.store.sync_timeout_secs);
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() >= timeout {
                if let serde_json::Value::Object(ref mut map) = response_json {
                    map.insert("embedding_status".to_string(), serde_json::json!("pending"));
                }
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match store.get(&memory.id).await {
                Ok(m) => {
                    if m.embedding_status == "complete" || m.embedding_status == "failed" {
                        if let serde_json::Value::Object(ref mut map) = response_json {
                            map.insert(
                                "embedding_status".to_string(),
                                serde_json::json!(m.embedding_status),
                            );
                        }
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    println!("{}", serde_json::to_string(&response_json)?);

    warn_if_no_daemon(store).await;
    Ok(())
}

/// Apply field projection to a search result JSON object.
///
/// When `fields` is None or empty, returns the object unchanged (backwards compatible).
/// When `fields` is Some(vec), returns only the keys listed in the vec.
/// Unknown field names are silently ignored — same behaviour as the MCP path in server.rs.
/// Apply field projection with one-level dot-notation support.
/// See server.rs apply_field_projection for full documentation.
fn apply_field_projection(
    obj: serde_json::Value,
    fields: &Option<Vec<String>>,
) -> serde_json::Value {
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
                        if child_key.contains('.') {
                            continue;
                        }
                        if let Some(serde_json::Value::Object(nested)) = map.get(parent_key) {
                            if let Some(child_val) = nested.get(child_key) {
                                let entry = result
                                    .entry(parent_key.to_string())
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
#[allow(clippy::too_many_arguments)] // CLI handler args map to command-line flags; struct refactor deferred
pub async fn cmd_search(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    query: String,
    limit: i64,
    created_after: Option<String>,
    created_before: Option<String>,
    tags: Option<Vec<String>>,
    source: Option<Vec<String>>,
    audience: Option<String>,
    type_hint: Option<String>,
    verbose: bool,
    json: bool,
    compact: bool,
    cursor: Option<String>,
    fields: Option<String>,
    min_salience: Option<f64>,
    project: Option<String>,
    depth: u8,
    tier: Option<String>,
    show_sources: bool,
) -> Result<()> {
    // Validate query size
    crate::validation::validate_query(&query, &config.input_limits)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let ca = created_after.as_deref().map(parse_datetime).transpose()?;
    let cb = created_before.as_deref().map(parse_datetime).transpose()?;

    // Parse tier filter
    let tier_filter: Option<Vec<String>> = match tier.as_deref() {
        None => None,
        Some("all") => Some(vec!["all".into()]),
        Some(list) => Some(list.split(',').map(|s| s.trim().to_string()).collect()),
    };

    // Validate min_salience and compute effective threshold (mirrors MCP path).
    if let Some(ms) = min_salience {
        if !(0.0..=1.0).contains(&ms) {
            return Err(anyhow::anyhow!("min_salience must be between 0.0 and 1.0"));
        }
    }
    let effective_min = min_salience
        .or(config.search.default_min_salience)
        .unwrap_or(0.0);

    // Parse comma-separated fields into a projection list.
    let field_list: Option<Vec<String>> = fields.map(|f| {
        f.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

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
    // When multi-model is configured, request per-tier embeddings for dual-query search.
    let tags_for_search = tags.clone().filter(|t| !t.is_empty());
    let fetch_limit = if cursor_position.is_some() {
        limit * 5
    } else {
        limit
    };

    let raw_hits = if config.embedding.is_multi_model() {
        // Multi-model path: request all-tier embeddings and use hybrid_search_multi_tier.
        match embed_multi_via_daemon(&query).await {
            Some(tier_vecs) => {
                // Convert Vec<f32> -> pgvector::Vector for each tier
                let tier_embeddings: std::collections::HashMap<String, pgvector::Vector> =
                    tier_vecs
                        .into_iter()
                        .map(|(tier, vec)| (tier, pgvector::Vector::from(vec)))
                        .collect();
                store
                    .hybrid_search_multi_tier(
                        &query,
                        &tier_embeddings,
                        fetch_limit,
                        ca,
                        cb,
                        tags_for_search.as_deref(),
                        Some(60.0), // bm25_k
                        Some(60.0), // vector_k
                        Some(40.0), // symbolic_k
                        source.as_deref(),
                        audience.as_deref(),
                        project.as_deref(),
                        tier_filter.clone(),
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            }
            None => {
                // Daemon offline — degrade to text-only search.
                eprintln!("warning: daemon offline — falling back to text-only search (results may be degraded). Start with: memcp daemon");
                store
                    .hybrid_search_multi_tier(
                        &query,
                        &std::collections::HashMap::new(),
                        fetch_limit,
                        ca,
                        cb,
                        tags_for_search.as_deref(),
                        Some(60.0),
                        None, // no vector leg
                        Some(40.0),
                        source.as_deref(),
                        audience.as_deref(),
                        project.as_deref(),
                        tier_filter,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            }
        }
    } else {
        // Single-model path (backward compat): single embed IPC + hybrid_search.
        let query_embedding_opt = embed_via_daemon(&query).await;
        let (query_embedding_vec, vector_k) = match &query_embedding_opt {
            Some(embedding) => {
                let vec = pgvector::Vector::from(embedding.clone());
                (Some(vec), Some(60.0_f64))
            }
            None => {
                eprintln!("warning: daemon offline — falling back to text-only search (results may be degraded). Start with: memcp daemon");
                (None, None)
            }
        };
        store
            .hybrid_search(
                &query,
                query_embedding_vec.as_ref(),
                fetch_limit,
                ca,
                cb,
                tags_for_search.as_deref(),
                Some(60.0), // bm25_k default
                vector_k,   // Some(60.0) when daemon alive, None when offline
                Some(40.0), // symbolic_k default
                source.as_deref(),
                audience.as_deref(),
                project.as_deref(),
                tier_filter,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?
    };

    // Apply type_hint filter post-search (symbolic leg doesn't filter by type_hint column).
    let raw_hits: Vec<_> = if let Some(ref th) = type_hint {
        raw_hits
            .into_iter()
            .filter(|h| h.memory.type_hint == *th)
            .collect()
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

    let scorer = SalienceScorer::new(&config.salience);
    scorer.rank(&mut scored_hits, &salience_inputs);

    // LLM re-ranking via daemon IPC (SCF-01 gap closure).
    // Same logic as server.rs:978-1030 — top 10 candidates, blend 0.7*llm + 0.3*salience.
    // Placed AFTER salience scoring and BEFORE cursor filtering (mirrors MCP serve pipeline).
    {
        let top_n = scored_hits.len().min(10);
        if top_n > 0 {
            let qi_config = config.query_intelligence.clone();
            let candidates: Vec<(String, String, usize)> = scored_hits[..top_n]
                .iter()
                .enumerate()
                .map(|(i, hit)| {
                    let content = if hit.memory.content.len() > qi_config.rerank_content_chars {
                        hit.memory.content[..qi_config.rerank_content_chars].to_string()
                    } else {
                        hit.memory.content.clone()
                    };
                    (hit.memory.id.clone(), content, i + 1)
                })
                .collect();

            if let Some(ranked) = rerank_via_daemon(&query, &candidates).await {
                // Blend: 0.7 * llm_rank_score + 0.3 * normalized_salience
                let max_s = scored_hits
                    .iter()
                    .map(|h| h.salience_score)
                    .fold(f64::MIN, f64::max);
                let min_s = scored_hits
                    .iter()
                    .map(|h| h.salience_score)
                    .fold(f64::MAX, f64::min);
                let range = (max_s - min_s).max(1e-6);

                for hit in scored_hits[..top_n].iter_mut() {
                    if let Some((_, llm_rank)) = ranked.iter().find(|(id, _)| id == &hit.memory.id)
                    {
                        let llm_score = 1.0 / (1.0 + *llm_rank as f64);
                        let norm_salience = (hit.salience_score - min_s) / range;
                        hit.salience_score = 0.7 * llm_score + 0.3 * norm_salience;
                    }
                }
                scored_hits[..top_n].sort_by(|a, b| {
                    b.salience_score
                        .partial_cmp(&a.salience_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            // If rerank_via_daemon returns None: daemon offline or no QI provider — silently skip (fail-open).
            // No additional warning — the existing embed-offline warning already covers degraded results.
        }
    }

    // Apply salience threshold filtering AFTER re-ranking, BEFORE cursor/take (mirrors MCP path).
    let mut scored_hits: Vec<ScoredHit> = if effective_min > 0.0 {
        scored_hits
            .into_iter()
            .filter(|h| h.salience_score >= effective_min)
            .collect()
    } else {
        scored_hits
    };

    // Compute composite relevance score (0-1) blending RRF and salience (mirrors MCP path).
    if scored_hits.len() == 1 {
        scored_hits[0].composite_score = 1.0;
    } else if scored_hits.len() > 1 {
        let max_rrf = scored_hits
            .iter()
            .map(|h| h.rrf_score)
            .fold(f64::MIN, f64::max);
        let min_rrf = scored_hits
            .iter()
            .map(|h| h.rrf_score)
            .fold(f64::MAX, f64::min);
        let rrf_range = (max_rrf - min_rrf).max(1e-9);

        let max_sal = scored_hits
            .iter()
            .map(|h| h.salience_score)
            .fold(f64::MIN, f64::max);
        let min_sal = scored_hits
            .iter()
            .map(|h| h.salience_score)
            .fold(f64::MAX, f64::min);
        let sal_range = (max_sal - min_sal).max(1e-9);

        for hit in &mut scored_hits {
            let norm_rrf = (hit.rrf_score - min_rrf) / rrf_range;
            let norm_sal = (hit.salience_score - min_sal) / sal_range;
            let ts = crate::config::tier_score_for(&hit.memory.knowledge_tier);
            hit.composite_score = 0.4 * norm_rrf + 0.4 * norm_sal + 0.2 * ts;
        }
    }

    // Phase 24.75: chunking removed; no parent/chunk dedup needed.

    // Apply cursor-based filtering: skip items at or before the cursor position.
    // Cursor encodes (salience_score, id) of the LAST item on the previous page.
    // Skip items where: score > last_score OR (score == last_score AND id <= last_id).
    let scored_hits: Vec<ScoredHit> = if let Some((last_score, ref last_id)) = cursor_position {
        scored_hits
            .into_iter()
            .filter(|h| {
                let score = h.salience_score;
                if (score - last_score).abs() < f64::EPSILON {
                    h.memory.id.as_str() > last_id.as_str()
                } else {
                    score < last_score
                }
            })
            .collect()
    } else {
        scored_hits
    };

    // Take limit items, detect if more remain.
    let has_more = scored_hits.len() as i64 > limit;
    let take = if has_more {
        limit as usize
    } else {
        scored_hits.len()
    };
    let scored_hits: Vec<ScoredHit> = scored_hits.into_iter().take(take).collect();

    // Build next_cursor from the last item's (salience_score, id) — keyset cursor.
    let next_cursor: Option<String> = if has_more {
        scored_hits
            .last()
            .map(|h| encode_search_keyset_cursor(h.salience_score, &h.memory.id))
    } else {
        None
    };

    // Pre-compute source chains if show_sources is requested (D-08: opt-in).
    let source_chains: std::collections::HashMap<String, Vec<(String, Memory)>> = if show_sources {
        let mut chains = std::collections::HashMap::new();
        for hit in &scored_hits {
            let sources = crate::transport::server::fetch_source_chain_single_hop(store, &hit.memory).await;
            if !sources.is_empty() {
                chains.insert(hit.memory.id.clone(), sources);
            }
        }
        chains
    } else {
        std::collections::HashMap::new()
    };

    // Format results according to output mode.
    if json {
        // --json: MCP-compatible JSON envelope. id always present at top level (SCF-03).
        let results: Vec<serde_json::Value> = scored_hits
            .iter()
            .map(|h| {
                let display_content = match depth {
                    0 => h
                        .memory
                        .abstract_text
                        .as_deref()
                        .unwrap_or(&h.memory.content),
                    1 => h
                        .memory
                        .overview_text
                        .as_deref()
                        .unwrap_or(&h.memory.content),
                    _ => &h.memory.content,
                };
                let abstract_available = h.memory.abstract_text.is_some();
                let mut entry = format_memory_json(&h.memory, true);
                if let Some(obj) = entry.as_object_mut() {
                    // Ensure id is always top-level (SCF-03)
                    obj.insert("id".to_string(), json!(h.memory.id));
                    obj.insert("content".to_string(), json!(display_content));
                    obj.insert("depth".to_string(), json!(depth));
                    obj.insert("abstract_available".to_string(), json!(abstract_available));
                    obj.insert("salience_score".to_string(), json!(h.salience_score));
                    obj.insert(
                        "composite_score".to_string(),
                        json!((h.composite_score * 1000.0).round() / 1000.0),
                    );
                    obj.insert("rrf_score".to_string(), json!(h.rrf_score));
                    obj.insert("match_source".to_string(), json!(h.match_source));
                    obj.insert("knowledge_tier".to_string(), json!(h.memory.knowledge_tier));
                    // Attach source chain if show_sources was requested
                    if let Some(sources) = source_chains.get(&h.memory.id) {
                        let source_entries: Vec<serde_json::Value> = sources
                            .iter()
                            .map(|(id, mem)| json!({
                                "id": id,
                                "content": crate::transport::server::truncate_source_content(&mem.content, 200),
                                "knowledge_tier": mem.knowledge_tier,
                            }))
                            .collect();
                        obj.insert("sources".to_string(), json!(source_entries));
                    }
                }
                // Apply field projection (no-op when fields is None or empty).
                apply_field_projection(entry, &field_list)
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
        // When --fields is specified, only the requested fields are included in the output.
        for h in &scored_hits {
            let compact_content = match depth {
                0 => h
                    .memory
                    .abstract_text
                    .as_deref()
                    .unwrap_or(&h.memory.content),
                1 => h
                    .memory
                    .overview_text
                    .as_deref()
                    .unwrap_or(&h.memory.content),
                _ => &h.memory.content,
            };
            if field_list.is_some() {
                // Projected compact: build full object then project and print as JSON.
                let mut entry = format_memory_json(&h.memory, true);
                if let Some(obj) = entry.as_object_mut() {
                    obj.insert("id".to_string(), json!(h.memory.id));
                    obj.insert("content".to_string(), json!(compact_content));
                    obj.insert("salience_score".to_string(), json!(h.salience_score));
                }
                let projected = apply_field_projection(entry, &field_list);
                println!("{}", serde_json::to_string(&projected)?);
            } else {
                let id_short = &h.memory.id[..8.min(h.memory.id.len())];
                let snippet: String = compact_content.chars().take(80).collect();
                let snippet = if compact_content.len() > 80 {
                    format!("{}…", snippet)
                } else {
                    snippet
                };
                let tags_str = h
                    .memory
                    .tags
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
                    "{} {:.3} {} ({}) [{}]",
                    id_short, h.salience_score, snippet, h.memory.source, tags_str
                );
            }
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
                let display_content = match depth {
                    0 => h
                        .memory
                        .abstract_text
                        .as_deref()
                        .unwrap_or(&h.memory.content),
                    1 => h
                        .memory
                        .overview_text
                        .as_deref()
                        .unwrap_or(&h.memory.content),
                    _ => &h.memory.content,
                };
                let abstract_available = h.memory.abstract_text.is_some();
                let mut entry = format_memory_json(&h.memory, verbose);
                if let Some(obj) = entry.as_object_mut() {
                    // Ensure id is always top-level (SCF-03)
                    obj.insert("id".to_string(), json!(h.memory.id));
                    obj.insert("content".to_string(), json!(display_content));
                    obj.insert("depth".to_string(), json!(depth));
                    obj.insert("abstract_available".to_string(), json!(abstract_available));
                    obj.insert("salience_score".to_string(), json!(h.salience_score));
                    obj.insert(
                        "composite_score".to_string(),
                        json!((h.composite_score * 1000.0).round() / 1000.0),
                    );
                    obj.insert("rrf_score".to_string(), json!(h.rrf_score));
                    obj.insert("match_source".to_string(), json!(h.match_source));
                    obj.insert("knowledge_tier".to_string(), json!(h.memory.knowledge_tier));
                    if let Some(sources) = source_chains.get(&h.memory.id) {
                        let source_entries: Vec<serde_json::Value> = sources
                            .iter()
                            .map(|(id, mem)| json!({
                                "id": id,
                                "content": crate::transport::server::truncate_source_content(&mem.content, 200),
                                "knowledge_tier": mem.knowledge_tier,
                            }))
                            .collect();
                        obj.insert("sources".to_string(), json!(source_entries));
                    }
                }
                // Apply field projection (no-op when fields is None or empty).
                apply_field_projection(entry, &field_list)
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
        project: None,
        session_id: None,
        agent_role: None,
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
        anyhow::anyhow!(
            "Invalid duration '{}'. Use format like '30m', '1h', '2h', '1d'",
            s
        )
    })?;

    match unit {
        "m" => Ok(chrono::Duration::minutes(num)),
        "h" => Ok(chrono::Duration::hours(num)),
        "d" => Ok(chrono::Duration::days(num)),
        _ => Err(anyhow::anyhow!(
            "Unknown duration unit '{}'. Use 'm' (minutes), 'h' (hours), or 'd' (days)",
            unit
        )),
    }
}

/// List memories with optional filters and cursor-based pagination.
#[allow(clippy::too_many_arguments)] // CLI handler args map to command-line flags; struct refactor deferred
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
    project: Option<String>,
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
        project,
        session_id: None,
        agent_role: None,
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
    let full_id = store
        .resolve_id_prefix(id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let memory = store
        .get(&full_id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!(
        "{}",
        serde_json::to_string(&format_memory_json(&memory, true))?
    );
    Ok(())
}

/// Delete a memory by ID (permanent).
pub async fn cmd_delete(store: &Arc<PostgresMemoryStore>, id: &str) -> Result<()> {
    let full_id = store
        .resolve_id_prefix(id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    store
        .delete(&full_id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("{}", serde_json::to_string(&json!({ "deleted": full_id }))?);
    Ok(())
}

/// Reinforce a memory to boost its salience.
pub async fn cmd_reinforce(store: &Arc<PostgresMemoryStore>, id: &str, rating: &str) -> Result<()> {
    let full_id = store
        .resolve_id_prefix(id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let row = store
        .reinforce_salience(&full_id, rating)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let output = json!({
        "memory_id": full_id,
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
/// Exposed as `pub` for external test access.
pub fn format_relative_time(dt: DateTime<Utc>) -> String {
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

    let (
        alive,
        daemon_info,
        last_ingest_at,
        ingest_count_today,
        watched_file_count,
        embedding_model,
        embedding_dimension,
        gc_info,
    ) = match daemon_row {
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
            (
                alive,
                info,
                last_ingest,
                ingest_today.unwrap_or(0),
                watched.unwrap_or(0),
                model,
                dimension,
                gc,
            )
        }
        _ => (
            false,
            json!({"alive": false}),
            None,
            0,
            0,
            None,
            None,
            json!({ "last_run_at": null, "pruned_total": 0, "dedup_merges": 0, "filter_stats": {} }),
        ),
    };

    // Pending work counts (exclude soft-deleted)
    let pending_embed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memories WHERE embedding_status = 'pending' AND deleted_at IS NULL",
    )
    .fetch_one(store.pool())
    .await
    .unwrap_or(0);

    let pending_extract: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memories WHERE extraction_status = 'pending' AND deleted_at IS NULL",
    )
    .fetch_one(store.pool())
    .await
    .unwrap_or(0);

    let total_memories: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL")
            .fetch_one(store.pool())
            .await
            .unwrap_or(0);

    // Deep health check (when --check is passed)
    let checks = if check {
        // 1. Database reachable (already connected, but verify with a ping)
        let db_ok = sqlx::query("SELECT 1")
            .fetch_one(store.pool())
            .await
            .is_ok();

        // 2. Ollama responding (if summarization is configured)
        let ollama_ok = if config.summarization.enabled {
            let url = format!("{}/api/version", config.summarization.ollama_base_url);
            reqwest::get(&url)
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
        } else {
            true // not configured = not a failure
        };

        // 3. Model cache present on disk
        let cache_dir = dirs::cache_dir()
            .unwrap_or_default()
            .join("fastembed_cache");
        let model_cache_ok = cache_dir.exists()
            && std::fs::read_dir(&cache_dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);

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

    // Curation stats
    let curation_stats = if config.curation.enabled {
        let runs = store.get_curation_runs(1).await.unwrap_or_default();
        let last_run = runs.first();
        json!({
            "enabled": true,
            "last_run": last_run.map(|r| r.completed_at.unwrap_or(r.started_at).to_rfc3339()),
            "last_run_status": last_run.map(|r| r.status.clone()),
            "last_merged": last_run.map(|r| r.merged_count).unwrap_or(0),
            "last_flagged": last_run.map(|r| r.flagged_stale_count).unwrap_or(0),
            "last_strengthened": last_run.map(|r| r.strengthened_count).unwrap_or(0),
            "interval_secs": config.curation.interval_secs,
        })
    } else {
        json!({"enabled": false})
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
        "curation": curation_stats,
    });
    if let Some(checks) = checks {
        if let Some(obj) = output.as_object_mut() {
            obj.insert("checks".to_string(), checks);
        }
    }

    Ok((
        output,
        alive,
        last_ingest_at,
        pending_embed as i32 + pending_extract as i32,
        total_memories as i32,
    ))
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
            let uptime_str = output
                .get("daemon")
                .and_then(|d| d.get("started_at"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .map(format_relative_time)
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

            println!(
                "{} daemon up {} | {} | {} | {} memories",
                icon, uptime_str, pending_str, ingest_str, total_memories
            );
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

        // Curation stats in pretty mode
        if let Some(curation) = output.get("curation") {
            let enabled = curation
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !enabled {
                println!("  Curation: disabled");
            } else if let Some(last_run) = curation.get("last_run").and_then(|v| v.as_str()) {
                let status = curation
                    .get("last_run_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let merged = curation
                    .get("last_merged")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let flagged = curation
                    .get("last_flagged")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let strengthened = curation
                    .get("last_strengthened")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let interval = curation
                    .get("interval_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let interval_str = if interval >= 86400 {
                    format!("every {}d", interval / 86400)
                } else if interval >= 3600 {
                    format!("every {}h", interval / 3600)
                } else {
                    format!("every {}s", interval)
                };
                println!(
                    "  Curation: last run {} ({}) | {} merged, {} flagged, {} strengthened | {}",
                    last_run, status, merged, flagged, strengthened, interval_str
                );
            } else {
                let interval = curation
                    .get("interval_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!("  Curation: enabled, no runs yet (interval: {}s)", interval);
            }
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
pub async fn cmd_feedback(store: &Arc<PostgresMemoryStore>, id: &str, signal: &str) -> Result<()> {
    let full_id = store
        .resolve_id_prefix(id)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    store
        .apply_feedback(&full_id, signal)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!(
        "{}",
        serde_json::to_string(&json!({ "ok": true, "id": full_id, "signal": signal }))?
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
// Recall
// ---------------------------------------------------------------------------

/// Recall relevant memories for automatic context injection.
///
/// Requires the daemon to be running (for query embedding via IPC).
///
/// Output format:
/// - `first=false`: `{ "session_id": "...", "count": N, "memories": [...] }`
/// - `first=true`:  `{ "current_datetime": "...", "preamble": "...", "session_id": "...", "count": N, "memories": [...] }`
///
/// Each memory in the array includes truncated content (configurable via `recall.truncation_chars`),
/// a `truncated` flag when content was cut, and `related_count`/`hint` when related memories exist.
///
/// The `--first` flag is designed for session start — injects preamble and datetime so agents
/// understand the memory system without repeating this overhead on every recall.
#[allow(clippy::too_many_arguments)] // CLI handler args map to command-line flags; struct refactor deferred
pub async fn cmd_recall(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    query: &str,
    session_id: Option<String>,
    reset: bool,
    project: Option<String>,
    first: bool,
    limit: Option<usize>,
    boost_tags: &[String],
    depth: u8,
    _tier: Option<String>,
    show_sources: bool,
) -> Result<()> {
    // Validate query size (recall query can be empty for query-less mode)
    if !query.is_empty() {
        crate::validation::validate_query(query, &config.input_limits)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
    }

    // Default preamble text. Tells agents how to use the memory system.
    const DEFAULT_PREAMBLE: &str = "You have access to persistent memory via memcp. \
        Key commands: `memcp store \"content\" --tags tag1,tag2` to save, \
        `memcp search \"query\"` to find, `memcp get <id>` for full content, \
        `memcp annotate --id <id> --tags tag1 --salience 1.5x` to enrich. \
        Memories persist across sessions. Store important decisions, preferences, and context.";

    let engine = crate::recall::RecallEngine::new(
        Arc::clone(store),
        config.recall.clone(),
        config.extraction.enabled,
    );

    let mut result = if query.is_empty() {
        // Query-less path — no embedding needed; ranked by salience + recency.
        engine
            .recall_queryless(
                session_id,
                reset,
                project.as_deref(),
                first,
                limit,
                boost_tags,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Recall failed: {}", e))?
    } else {
        // Query-based path — embed via daemon for vector similarity.
        let query_embedding = match embed_via_daemon(query).await {
            Some(emb) => emb,
            None => {
                eprintln!("error: recall requires the daemon to be running for query embedding");
                eprintln!("hint: start the daemon with 'memcp daemon start'");
                std::process::exit(1);
            }
        };

        let r = engine
            .recall(
                &query_embedding,
                session_id,
                reset,
                project.as_deref(),
                boost_tags,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Recall failed: {}", e))?;
        r
    };

    // For query-based path with first=true, fetch project summary separately.
    // (recall_queryless already handles this internally via the first parameter.)
    if !query.is_empty() && first && result.summary.is_none() {
        result.summary = match store.fetch_project_summary(project.as_deref()).await {
            Ok(Some((id, content))) => Some(crate::recall::RecalledMemory {
                memory_id: id,
                content,
                relevance: 1.0,
                boost_applied: false,
                boost_score: 0.0,
                trust_level: 1.0,
                abstract_text: None,
                overview_text: None,
                knowledge_tier: Some("explicit".to_string()),
                source_ids: None,
            }),
            Ok(None) => None,
            Err(e) => {
                tracing::warn!(error = %e, "fetch_project_summary failed — skipping");
                None
            }
        };
    }

    // Collect memory IDs for related context lookup.
    let memory_ids: Vec<String> = result
        .memories
        .iter()
        .map(|m| m.memory_id.clone())
        .collect();

    // Fetch related context (batch) if enabled and there are memories.
    let related_map = if config.recall.related_context_enabled && !memory_ids.is_empty() {
        match store.get_related_context(&memory_ids).await {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!(error = %e, "get_related_context failed — skipping related hints");
                std::collections::HashMap::new()
            }
        }
    } else {
        std::collections::HashMap::new()
    };

    let truncation_chars = config.recall.truncation_chars;

    // Build the memories array with truncation and related context.
    let memories: Vec<serde_json::Value> = result
        .memories
        .iter()
        .map(|mem| {
            // Apply depth selection before truncation.
            let source_content = match depth {
                0 => mem.abstract_text.as_deref().unwrap_or(&mem.content),
                1 => mem.overview_text.as_deref().unwrap_or(&mem.content),
                _ => &mem.content,
            };
            let (truncated_content, was_truncated) =
                truncate_content(source_content, truncation_chars);

            let mut obj = json!({
                "id": mem.memory_id,
                "content": truncated_content,
                "relevance": mem.relevance,
            });

            if was_truncated {
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert("truncated".to_string(), json!(true));
                }
            }

            // Include boost metadata when boost was applied (mirrors serde skip_serializing_if).
            if mem.boost_applied {
                if let serde_json::Value::Object(ref mut map) = obj {
                    map.insert("boost_applied".to_string(), json!(true));
                    map.insert("boost_score".to_string(), json!(mem.boost_score));
                }
            }

            // Add related context if present and count > 0.
            if let Some(related) = related_map.get(&mem.memory_id) {
                if related.related_count > 0 {
                    let hint = build_related_hint(&related.shared_tags);
                    if let serde_json::Value::Object(ref mut map) = obj {
                        map.insert("related_count".to_string(), json!(related.related_count));
                        if !hint.is_empty() {
                            map.insert("hint".to_string(), json!(hint));
                        }
                    }
                }
            }

            obj
        })
        .collect();

    // Attach source chain if show_sources is requested (D-08: opt-in).
    let mut memories = memories;
    if show_sources {
        let mem_ids: Vec<String> = result.memories.iter().map(|m| m.memory_id.clone()).collect();
        if let Ok(full_memories) = store.get_memories_by_ids(&mem_ids).await {
            for (i, recalled) in result.memories.iter().enumerate() {
                if let Some(full_mem) = full_memories.get(&recalled.memory_id) {
                    let sources = crate::transport::server::fetch_source_chain_single_hop(store, full_mem).await;
                    if !sources.is_empty() {
                        let source_entries: Vec<serde_json::Value> = sources
                            .iter()
                            .map(|(id, mem)| {
                                json!({
                                    "id": id,
                                    "content": crate::transport::server::truncate_source_content(&mem.content, 200),
                                    "knowledge_tier": mem.knowledge_tier,
                                })
                            })
                            .collect();
                        if let Some(obj) = memories.get_mut(i).and_then(|v| v.as_object_mut()) {
                            obj.insert("sources".to_string(), json!(source_entries));
                        }
                    }
                }
            }
        }
    }

    // Assemble final output.
    let mut output = json!({
        "session_id": result.session_id,
        "count": result.count,
        "memories": memories,
    });

    // Add summary if present (query-less with first=true, or query-based with first=true).
    if let Some(ref summary) = result.summary {
        if let serde_json::Value::Object(ref mut map) = output {
            map.insert(
                "summary".to_string(),
                json!({
                    "id": summary.memory_id,
                    "content": summary.content,
                }),
            );
        }
    }

    if first {
        let preamble = config
            .recall
            .preamble_override
            .as_deref()
            .unwrap_or(DEFAULT_PREAMBLE);

        if let serde_json::Value::Object(ref mut map) = output {
            map.insert(
                "current_datetime".to_string(),
                json!(Utc::now().to_rfc3339()),
            );
            map.insert("preamble".to_string(), json!(preamble));
        }
    }

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

// ---------------------------------------------------------------------------
// Discover command
// ---------------------------------------------------------------------------

/// Discover unexpected memory connections in the cosine sweet spot.
///
/// Requires the daemon to be running (embedding via IPC).
/// Results include similarity score and — when LLM explanations are not available —
/// just the memory content and score.
pub async fn cmd_discover(
    store: &Arc<PostgresMemoryStore>,
    query: String,
    min_similarity: f64,
    max_similarity: f64,
    limit: u32,
    project: Option<String>,
    json_output: bool,
) -> Result<()> {
    if query.trim().is_empty() {
        anyhow::bail!("query is required and must not be empty");
    }

    if min_similarity >= max_similarity {
        anyhow::bail!(
            "min_similarity ({}) must be less than max_similarity ({})",
            min_similarity,
            max_similarity
        );
    }

    // Embed via daemon for vector similarity
    let query_embedding = match embed_via_daemon(&query).await {
        Some(emb) => emb,
        None => {
            eprintln!("error: discover requires the daemon to be running for query embedding");
            eprintln!("hint: start the daemon with 'memcp daemon'");
            std::process::exit(1);
        }
    };

    let query_vec = pgvector::Vector::from(query_embedding);
    let results = store
        .discover_associations(
            &query_vec,
            min_similarity,
            max_similarity,
            limit,
            project.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Discovery failed: {}", e))?;

    if json_output {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|(memory, sim)| {
                json!({
                    "id": memory.id,
                    "content": memory.content,
                    "type_hint": memory.type_hint,
                    "tags": memory.tags,
                    "similarity": format!("{:.3}", sim),
                    "created_at": memory.created_at.to_rfc3339(),
                    "project": memory.project,
                })
            })
            .collect();
        let output = json!({
            "discoveries": items,
            "query": query,
            "similarity_range": [min_similarity, max_similarity],
            "count": items.len(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if results.is_empty() {
            println!("No discoveries found for \"{query}\" in similarity range [{min_similarity:.1}, {max_similarity:.1}].");
            return Ok(());
        }

        println!(
            "Discovered {} connection(s) for \"{query}\":",
            results.len()
        );
        println!();

        for (i, (memory, sim)) in results.iter().enumerate() {
            println!("{}. [{:.2}] {}", i + 1, sim, memory.content);
            if let Some(ref tags) = memory.tags {
                if let Some(arr) = tags.as_array() {
                    let tag_str: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    if !tag_str.is_empty() {
                        println!("   Tags: {}", tag_str.join(", "));
                    }
                }
            }
            println!();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Curation commands
// ---------------------------------------------------------------------------

/// Run a single curation pass.
///
/// With `--propose`: previews what actions would be taken without executing.
/// Without `--propose`: executes the full curation pipeline (merge, flag, strengthen).
pub async fn cmd_curation_run(
    store: &Arc<PostgresMemoryStore>,
    config: &Config,
    propose: bool,
) -> Result<()> {
    use crate::curation;

    let curation_config = config.curation.clone();

    // Create LLM provider if configured
    let provider = match curation::create_curation_provider(&curation_config) {
        Ok(p) => p,
        Err(e) => {
            let output = serde_json::json!({
                "status": "error",
                "error": format!("Failed to create curation provider: {}", e),
            });
            println!("{}", serde_json::to_string(&output)?);
            return Ok(());
        }
    };

    let provider_ref = provider.as_deref();
    let result = curation::worker::run_curation(store, &curation_config, provider_ref, propose)
        .await
        .map_err(|e| anyhow::anyhow!("Curation failed: {}", e))?;

    if let Some(reason) = &result.skipped_reason {
        let output = serde_json::json!({
            "status": "skipped",
            "reason": reason,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else if propose && !result.proposed_actions.is_empty() {
        let output = serde_json::json!({
            "status": "proposed",
            "candidates_processed": result.candidates_processed,
            "clusters_found": result.clusters_found,
            "proposed_merges": result.merged_count,
            "proposed_stale_flags": result.flagged_stale_count,
            "proposed_strengthens": result.strengthened_count,
            "proposed_skips": result.skipped_count,
            "actions": result.proposed_actions,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if propose {
        let output = serde_json::json!({
            "status": "proposed",
            "candidates_processed": result.candidates_processed,
            "clusters_found": result.clusters_found,
            "proposed_merges": 0,
            "proposed_stale_flags": 0,
            "proposed_strengthens": 0,
            "proposed_skips": 0,
            "actions": [],
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let output = serde_json::json!({
            "status": "ok",
            "run_id": result.run_id,
            "candidates_processed": result.candidates_processed,
            "clusters_found": result.clusters_found,
            "merged": result.merged_count,
            "flagged_stale": result.flagged_stale_count,
            "strengthened": result.strengthened_count,
            "skipped": result.skipped_count,
        });
        println!("{}", serde_json::to_string(&output)?);
    }

    Ok(())
}

/// Show curation run history.
///
/// Displays recent curation runs with their action counts and status.
pub async fn cmd_curation_log(
    store: &Arc<PostgresMemoryStore>,
    limit: i64,
    run_id: Option<String>,
) -> Result<()> {
    // If a specific run_id is provided, show detailed actions for that run
    if let Some(ref rid) = run_id {
        let actions = store
            .get_curation_actions(rid)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to fetch curation actions: {}", e))?;

        if actions.is_empty() {
            let output = serde_json::json!({
                "run_id": rid,
                "actions": [],
                "message": "No actions found for this run",
            });
            println!("{}", serde_json::to_string(&output)?);
            return Ok(());
        }

        let action_json: Vec<serde_json::Value> = actions
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "action_type": a.action_type,
                    "target_memory_ids": a.target_memory_ids,
                    "merged_memory_id": a.merged_memory_id,
                    "original_salience": a.original_salience,
                    "details": a.details,
                    "created_at": a.created_at,
                })
            })
            .collect();

        let output = serde_json::json!({
            "run_id": rid,
            "actions": action_json,
            "count": actions.len(),
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    // Default: show recent runs
    let runs = store
        .get_curation_runs(limit as usize)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch curation runs: {}", e))?;

    if runs.is_empty() {
        let output = serde_json::json!({
            "runs": [],
            "message": "No curation runs found",
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    let run_json: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "status": r.status,
                "mode": r.mode,
                "started_at": r.started_at,
                "completed_at": r.completed_at,
                "merged_count": r.merged_count,
                "flagged_stale_count": r.flagged_stale_count,
                "strengthened_count": r.strengthened_count,
                "skipped_count": r.skipped_count,
                "error_message": r.error_message,
            })
        })
        .collect();

    let output = serde_json::json!({
        "runs": run_json,
        "count": runs.len(),
    });
    println!("{}", serde_json::to_string(&output)?);

    Ok(())
}

/// Undo a curation run.
///
/// Reverses all actions from a completed run: restores soft-deleted originals,
/// removes merged memories, reverts stability changes, and strips added tags.
pub async fn cmd_curation_undo(store: &Arc<PostgresMemoryStore>, run_id: &str) -> Result<()> {
    let undo_count = store
        .undo_curation_run(run_id)
        .await
        .map_err(|e| anyhow::anyhow!("Undo failed: {}", e))?;

    let output = serde_json::json!({
        "status": "ok",
        "run_id": run_id,
        "actions_undone": undo_count,
    });
    println!("{}", serde_json::to_string(&output)?);

    Ok(())
}

// ---------------------------------------------------------------------------
// Annotate
// ---------------------------------------------------------------------------

/// Result of an annotate operation — used by both CLI and MCP code paths.
pub struct AnnotateResult {
    pub id: String,
    pub tags_added: Vec<String>,
    pub tags_removed: Vec<String>,
    pub salience_before: Option<f64>,
    pub salience_after: Option<f64>,
}

/// Core annotate logic shared by cmd_annotate (CLI) and annotate_memory (MCP).
///
/// 1. Verifies the memory exists.
/// 2. Applies tag append or replace.
/// 3. Applies salience absolute or multiplier.
/// 4. Returns a diff describing what changed.
pub async fn annotate_logic(
    store: &Arc<PostgresMemoryStore>,
    id: &str,
    tags_to_append: Option<Vec<String>>,
    tags_to_replace: Option<Vec<String>>,
    salience_input: Option<String>,
) -> Result<AnnotateResult> {
    // 1. Verify memory exists and capture current state.
    let memory = store
        .get(id)
        .await
        .map_err(|e| anyhow::anyhow!("Memory not found: {}", e))?;

    // 2. Compute tag changes.
    // Memory.tags is Option<serde_json::Value> (JSONB array) — extract to Vec<String>.
    let tags_before: Vec<String> = memory
        .tags
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let new_tags: Vec<String> = if let Some(replace) = tags_to_replace {
        // replace_tags wins over tags
        replace
    } else if let Some(append) = tags_to_append {
        // merge + deduplicate + sort
        let mut set: HashSet<String> = tags_before.iter().cloned().collect();
        for t in append {
            set.insert(t);
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    } else {
        tags_before.clone()
    };

    let tags_changed = new_tags != tags_before;
    if tags_changed {
        store
            .update(
                id,
                UpdateMemory {
                    content: None,
                    type_hint: None,
                    tags: Some(new_tags.clone()),
                    source: None,
                    trust_level: None,
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update tags: {}", e))?;
    }

    // 3. Compute salience changes.
    let (salience_before, salience_after) = if let Some(ref input) = salience_input {
        let salience_map = store
            .get_salience_data(&[id.to_string()])
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read salience: {}", e))?;
        let current = salience_map.get(id).cloned().unwrap_or_default();

        let new_stability = if input.ends_with('x') {
            // Multiplier mode
            let prefix = &input[..input.len() - 1];
            let multiplier: f64 = prefix
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid salience multiplier: '{}'", input))?;
            (current.stability * multiplier).min(100.0)
        } else {
            // Absolute mode
            input
                .parse::<f64>()
                .map_err(|_| anyhow::anyhow!("Invalid salience value: '{}'", input))?
        };

        store
            .upsert_salience(
                id,
                new_stability,
                current.difficulty,
                current.reinforcement_count,
                current.last_reinforced_at,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to upsert salience: {}", e))?;

        (Some(current.stability), Some(new_stability))
    } else {
        (None, None)
    };

    // 4. Compute tag diff.
    let before_set: HashSet<&String> = tags_before.iter().collect();
    let after_set: HashSet<&String> = new_tags.iter().collect();
    let mut tags_added: Vec<String> = after_set
        .difference(&before_set)
        .map(|s| (*s).clone())
        .collect();
    tags_added.sort();
    let mut tags_removed: Vec<String> = before_set
        .difference(&after_set)
        .map(|s| (*s).clone())
        .collect();
    tags_removed.sort();

    Ok(AnnotateResult {
        id: id.to_string(),
        tags_added,
        tags_removed,
        salience_before,
        salience_after,
    })
}

/// CLI handler for `memcp annotate`.
pub async fn cmd_annotate(
    store: &Arc<PostgresMemoryStore>,
    id: &str,
    tags_to_append: Option<Vec<String>>,
    tags_to_replace: Option<Vec<String>>,
    salience_input: Option<String>,
) -> Result<()> {
    let result = annotate_logic(store, id, tags_to_append, tags_to_replace, salience_input).await?;

    let mut changes = serde_json::Map::new();
    changes.insert("tags_added".to_string(), json!(result.tags_added));
    changes.insert("tags_removed".to_string(), json!(result.tags_removed));
    if let (Some(before), Some(after)) = (result.salience_before, result.salience_after) {
        changes.insert("salience_before".to_string(), json!(before));
        changes.insert("salience_after".to_string(), json!(after));
    }

    let output = json!({
        "id": result.id,
        "changes": changes,
    });
    println!("{}", serde_json::to_string(&output)?);
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
