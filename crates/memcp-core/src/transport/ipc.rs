//! IPC channel between CLI and daemon for embedding queries and LLM re-ranking.
//!
//! The daemon holds the loaded fastembed model (87MB, too heavy to reload per CLI invocation)
//! and the optional QI reranking provider (LLM-based, also too expensive to initialize per call).
//! This module provides:
//!   - `embed_socket_path()` — well-known Unix domain socket path
//!   - `start_embed_listener()` — daemon-side: binds socket, serves embed + rerank requests
//!   - `embed_via_daemon()` — CLI-side: connects, sends text, receives embedding vector
//!   - `embed_multi_via_daemon()` — CLI-side: connects, sends text, receives all-tier embeddings
//!   - `rerank_via_daemon()` — CLI-side: connects, sends candidates, receives ranked results
//!
//! Protocol: newline-delimited JSON over a Unix domain socket.
//!   Embed request:        {"text": "query text"}
//!   Embed response:       {"embedding": [0.1, 0.2, ...]}  or  {"error": "message"}
//!
//!   Embed-multi request:  {"type":"embed_multi","text":"query text"}
//!   Embed-multi response: {"embeddings": {"fast": [0.1,...], "quality": [0.2,...]}}
//!                      or {"error": "message"}
//!
//!   Rerank request:  {"type":"rerank","query":"...","candidates":[{"id":"uuid","content":"text","current_rank":1},...]}
//!   Rerank response: {"ranked":[{"id":"uuid","llm_rank":1},...]}
//!                or  {"error":"message"}
//!                or  {"noop":true}  (when no QI provider configured)
//!
//! Design: fail-open. CLI callers receive `None` on any error (timeout, connection
//! refused, socket absent, parse error) and fall back gracefully.
//! This matches the fail-open pattern used throughout memcp.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::embedding::EmbeddingProvider;
use crate::embedding::router::EmbeddingRouter;
use crate::query_intelligence::{QueryIntelligenceProvider, RankedCandidate};
use crate::store::postgres::PostgresMemoryStore;

// ---------------------------------------------------------------------------
// Socket path
// ---------------------------------------------------------------------------

/// Returns the canonical path for the embed IPC socket.
///
/// Path: `$XDG_DATA_HOME/memcp/embed.sock` (usually `~/.local/share/memcp/embed.sock`).
/// The parent directory is created if it does not already exist.
pub fn embed_socket_path() -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".local")
                .join("share")
        });
    let dir = base.join("memcp");
    // Best-effort: create parent if missing. Ignore errors; the bind will surface them.
    let _ = std::fs::create_dir_all(&dir);
    dir.join("embed.sock")
}

// ---------------------------------------------------------------------------
// Daemon-side listener
// ---------------------------------------------------------------------------

/// Spawn the IPC listener as a background task.
///
/// Handles embedding requests ({"text":"..."}), multi-tier embed requests
/// ({"type":"embed_multi","text":"..."}), and reranking requests ({"type":"rerank",...})
/// over the same Unix domain socket.
///
/// Called by `run_daemon()` alongside existing worker spawns. The listener
/// accepts incoming connections, each handled in an independent tokio task.
///
/// When `multi_tier` is `Some((router, store))`, the listener also handles
/// `embed_multi` requests that return per-tier embeddings for dual-query search.
///
/// **Stale socket handling (Pitfall 5):** Before binding, we attempt a connect.
/// If connection is refused, the socket is stale — remove it and re-bind.
/// This is standard Unix daemon practice.
pub async fn start_embed_listener(
    socket_path: PathBuf,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    qi_provider: Option<Arc<dyn QueryIntelligenceProvider + Send + Sync>>,
    multi_tier: Option<(Arc<EmbeddingRouter>, Arc<PostgresMemoryStore>)>,
) {
    // Remove stale socket if it exists but is not listening.
    if socket_path.exists() {
        let stale = UnixStream::connect(&socket_path).await.is_err();
        if stale {
            if let Err(e) = std::fs::remove_file(&socket_path) {
                tracing::warn!(path = %socket_path.display(), error = %e, "Failed to remove stale embed socket");
            }
        } else {
            // Another daemon is already listening — don't steal the socket.
            tracing::warn!(
                path = %socket_path.display(),
                "Embed socket already in use — skipping embed IPC listener"
            );
            return;
        }
    }

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(path = %socket_path.display(), error = %e, "Failed to bind embed IPC socket");
            return;
        }
    };

    tracing::info!(path = %socket_path.display(), "Embed IPC listening on {}", socket_path.display());

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "Embed IPC accept error");
                continue;
            }
        };

        let provider = provider.clone();
        let qi_provider = qi_provider.clone();
        let multi_tier = multi_tier.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_ipc_connection(stream, provider, qi_provider, multi_tier).await {
                tracing::debug!(error = %e, "IPC connection error");
            }
        });
    }
}

/// Handle a single IPC connection.
///
/// Dispatches on the `"type"` field:
/// - No `"type"` field or `"type":"embed"` → single embedding request (backward compatible)
/// - `"type":"embed_multi"` → multi-tier embedding request (returns all-tier embeddings)
/// - `"type":"rerank"` → LLM re-ranking request
async fn handle_ipc_connection(
    mut stream: UnixStream,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
    qi_provider: Option<Arc<dyn QueryIntelligenceProvider + Send + Sync>>,
    multi_tier: Option<(Arc<EmbeddingRouter>, Arc<PostgresMemoryStore>)>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    reader.read_line(&mut line).await?;
    let line = line.trim();

    if line.is_empty() {
        return Ok(());
    }

    let request: serde_json::Value = serde_json::from_str(line)?;

    let response = match request.get("type").and_then(|t| t.as_str()) {
        Some("rerank") => {
            handle_rerank_request(&request, qi_provider.as_deref()).await
        }
        Some("embed_multi") => {
            handle_embed_multi_request(&request, multi_tier.as_ref()).await
        }
        // No "type" field (legacy embed request) or "type":"embed"
        _ => {
            handle_embed_request(&request, provider).await
        }
    };

    let mut response_line = serde_json::to_string(&response)?;
    response_line.push('\n');
    write_half.write_all(response_line.as_bytes()).await?;
    write_half.flush().await?;

    Ok(())
}

/// Handle an embedding request ({"text":"..."}).
async fn handle_embed_request(
    request: &serde_json::Value,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
) -> serde_json::Value {
    let text = match request["text"].as_str() {
        Some(t) => t,
        None => {
            return serde_json::json!({ "error": "Missing 'text' field in embed request" });
        }
    };

    match provider.embed(text).await {
        Ok(embedding) => serde_json::json!({ "embedding": embedding }),
        Err(e) => {
            tracing::warn!(error = %e, "Embedding failed in IPC handler");
            serde_json::json!({ "error": e.to_string() })
        }
    }
}

/// Handle a multi-tier embed request ({"type":"embed_multi","text":"..."}).
///
/// Returns `{"embeddings": {"fast": [...], "quality": [...]}}` with one entry per
/// active tier. Tiers with zero embeddings in the corpus are skipped (lazy optimization).
///
/// Falls back to `{"embeddings": {"fast": [...]}}` (single-tier) when the router
/// is not available (single-model daemon) or when non-default tiers have no data.
async fn handle_embed_multi_request(
    request: &serde_json::Value,
    multi_tier: Option<&(Arc<EmbeddingRouter>, Arc<PostgresMemoryStore>)>,
) -> serde_json::Value {
    let text = match request["text"].as_str() {
        Some(t) => t,
        None => {
            return serde_json::json!({ "error": "Missing 'text' field in embed_multi request" });
        }
    };

    match multi_tier {
        Some((router, store)) => {
            match router.embed_query_all_tiers(text, store).await {
                Ok(tier_embeddings) => {
                    // Convert HashMap<String, pgvector::Vector> to JSON
                    let embeddings_json: serde_json::Map<String, serde_json::Value> =
                        tier_embeddings
                            .into_iter()
                            .map(|(tier, vec)| {
                                let floats: Vec<f32> = vec.to_vec();
                                (tier, serde_json::json!(floats))
                            })
                            .collect();
                    serde_json::json!({ "embeddings": embeddings_json })
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Multi-tier embedding failed in IPC handler");
                    serde_json::json!({ "error": e.to_string() })
                }
            }
        }
        None => {
            // Single-model daemon: no router available — return error so CLI degrades gracefully
            serde_json::json!({ "error": "Multi-tier embedding not available (single-model daemon)" })
        }
    }
}

/// Handle a rerank request ({"type":"rerank","query":"...","candidates":[...]}).
///
/// Returns {"ranked":[...]} on success, {"noop":true} if no QI provider,
/// {"error":"..."} on failure.
async fn handle_rerank_request(
    request: &serde_json::Value,
    qi_provider: Option<&(dyn QueryIntelligenceProvider + Send + Sync)>,
) -> serde_json::Value {
    let provider = match qi_provider {
        Some(p) => p,
        None => return serde_json::json!({ "noop": true }),
    };

    let query = match request["query"].as_str() {
        Some(q) => q,
        None => return serde_json::json!({ "error": "Missing 'query' field in rerank request" }),
    };

    let candidates_json = match request["candidates"].as_array() {
        Some(c) => c,
        None => return serde_json::json!({ "error": "Missing 'candidates' field in rerank request" }),
    };

    let mut candidates = Vec::with_capacity(candidates_json.len());
    for c in candidates_json {
        let id = match c["id"].as_str() {
            Some(id) => id.to_string(),
            None => return serde_json::json!({ "error": "Candidate missing 'id' field" }),
        };
        let content = match c["content"].as_str() {
            Some(content) => content.to_string(),
            None => return serde_json::json!({ "error": "Candidate missing 'content' field" }),
        };
        let current_rank = c["current_rank"].as_u64().unwrap_or(1) as usize;
        candidates.push(RankedCandidate { id, content, current_rank });
    }

    match provider.rerank(query, &candidates).await {
        Ok(ranked) => {
            let ranked_json: Vec<serde_json::Value> = ranked
                .iter()
                .map(|r| serde_json::json!({ "id": r.id, "llm_rank": r.llm_rank }))
                .collect();
            serde_json::json!({ "ranked": ranked_json })
        }
        Err(e) => {
            tracing::warn!(error = %e, "Reranking failed in IPC handler");
            serde_json::json!({ "error": e.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------
// CLI-side client: embed
// ---------------------------------------------------------------------------

/// Attempt to obtain an embedding vector from the running daemon via IPC.
///
/// Returns `None` on any failure (daemon offline, timeout, parse error).
/// The caller should fall back to text-only search and warn to stderr.
///
/// Timeout: 500ms (Pitfall 1 — avoid hanging CLI).
pub async fn embed_via_daemon(text: &str) -> Option<Vec<f32>> {
    let socket_path = embed_socket_path();

    // Attempt connection with 500ms timeout.
    let stream = tokio::time::timeout(
        Duration::from_millis(500),
        UnixStream::connect(&socket_path),
    )
    .await
    .ok()?  // timeout expired
    .ok()?; // connection refused or socket absent

    send_embed_request(stream, text).await
}

/// Send an embed request and parse the response.
async fn send_embed_request(mut stream: UnixStream, text: &str) -> Option<Vec<f32>> {
    let request = serde_json::json!({ "text": text });
    let mut request_line = serde_json::to_string(&request).ok()?;
    request_line.push('\n');

    // Write with timeout.
    tokio::time::timeout(
        Duration::from_millis(500),
        stream.write_all(request_line.as_bytes()),
    )
    .await
    .ok()?
    .ok()?;

    stream.flush().await.ok()?;

    // Read response with timeout.
    let (read_half, _) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut response_line = String::new();

    tokio::time::timeout(
        Duration::from_millis(500),
        reader.read_line(&mut response_line),
    )
    .await
    .ok()?
    .ok()?;

    let response: serde_json::Value = serde_json::from_str(response_line.trim()).ok()?;

    // Return None on error responses.
    if response.get("error").is_some() {
        return None;
    }

    let embedding = response["embedding"].as_array()?;
    let floats: Vec<f32> = embedding
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();

    if floats.is_empty() {
        None
    } else {
        Some(floats)
    }
}

/// Attempt to obtain per-tier embedding vectors from the running daemon via IPC.
///
/// Returns `None` on any failure (daemon offline, timeout, parse error, single-model daemon).
/// The caller should fall back to `embed_via_daemon()` for single-embedding search.
///
/// Timeout: 500ms per request (same as single embed).
pub async fn embed_multi_via_daemon(text: &str) -> Option<HashMap<String, Vec<f32>>> {
    let socket_path = embed_socket_path();

    let stream = tokio::time::timeout(
        Duration::from_millis(500),
        UnixStream::connect(&socket_path),
    )
    .await
    .ok()?
    .ok()?;

    send_embed_multi_request(stream, text).await
}

/// Send an embed_multi request and parse the response.
async fn send_embed_multi_request(mut stream: UnixStream, text: &str) -> Option<HashMap<String, Vec<f32>>> {
    let request = serde_json::json!({ "type": "embed_multi", "text": text });
    let mut request_line = serde_json::to_string(&request).ok()?;
    request_line.push('\n');

    tokio::time::timeout(
        Duration::from_millis(500),
        stream.write_all(request_line.as_bytes()),
    )
    .await
    .ok()?
    .ok()?;

    stream.flush().await.ok()?;

    let (read_half, _) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut response_line = String::new();

    tokio::time::timeout(
        Duration::from_millis(500),
        reader.read_line(&mut response_line),
    )
    .await
    .ok()?
    .ok()?;

    let response: serde_json::Value = serde_json::from_str(response_line.trim()).ok()?;

    if response.get("error").is_some() {
        return None;
    }

    let embeddings_map = response["embeddings"].as_object()?;
    let mut result: HashMap<String, Vec<f32>> = HashMap::new();
    for (tier, vec_json) in embeddings_map {
        let floats: Vec<f32> = vec_json
            .as_array()?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();
        if !floats.is_empty() {
            result.insert(tier.clone(), floats);
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

// ---------------------------------------------------------------------------
// CLI-side client: rerank
// ---------------------------------------------------------------------------

/// Attempt to re-rank search candidates via the running daemon's LLM provider.
///
/// Returns `None` on any failure (daemon offline, timeout, noop — no QI provider configured,
/// connection refused). Callers should silently skip re-ranking and use salience order.
///
/// Timeout: 5000ms (reranking involves an LLM call, much slower than embedding).
///
/// # Arguments
/// - `query` — the original search query
/// - `candidates` — `(id, content, current_rank)` tuples (1-indexed rank)
///
/// # Returns
/// - `Some(Vec<(id, llm_rank)>)` — reranked IDs with their new 1-indexed ranks
/// - `None` — daemon offline, no QI provider, or any error (fail-open)
pub async fn rerank_via_daemon(
    query: &str,
    candidates: &[(String, String, usize)],
) -> Option<Vec<(String, usize)>> {
    if candidates.is_empty() {
        return None;
    }

    let socket_path = embed_socket_path();

    // Attempt connection with 5000ms timeout (LLM calls are slow).
    let stream = tokio::time::timeout(
        Duration::from_millis(5000),
        UnixStream::connect(&socket_path),
    )
    .await
    .ok()?
    .ok()?;

    send_rerank_request(stream, query, candidates).await
}

/// Send a rerank request and parse the response.
async fn send_rerank_request(
    mut stream: UnixStream,
    query: &str,
    candidates: &[(String, String, usize)],
) -> Option<Vec<(String, usize)>> {
    let candidates_json: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(id, content, rank)| {
            serde_json::json!({
                "id": id,
                "content": content,
                "current_rank": rank
            })
        })
        .collect();

    let request = serde_json::json!({
        "type": "rerank",
        "query": query,
        "candidates": candidates_json,
    });

    let mut request_line = serde_json::to_string(&request).ok()?;
    request_line.push('\n');

    // Write with 5000ms timeout.
    tokio::time::timeout(
        Duration::from_millis(5000),
        stream.write_all(request_line.as_bytes()),
    )
    .await
    .ok()?
    .ok()?;

    stream.flush().await.ok()?;

    // Read response with 5000ms timeout.
    let (read_half, _) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut response_line = String::new();

    tokio::time::timeout(
        Duration::from_millis(5000),
        reader.read_line(&mut response_line),
    )
    .await
    .ok()?
    .ok()?;

    let response: serde_json::Value = serde_json::from_str(response_line.trim()).ok()?;

    // noop: daemon running but no QI provider configured — silently return None.
    if response.get("noop").is_some() {
        return None;
    }

    // error: LLM call failed — return None (fail-open).
    if response.get("error").is_some() {
        return None;
    }

    let ranked = response["ranked"].as_array()?;
    let results: Vec<(String, usize)> = ranked
        .iter()
        .filter_map(|r| {
            let id = r["id"].as_str()?.to_string();
            let llm_rank = r["llm_rank"].as_u64()? as usize;
            Some((id, llm_rank))
        })
        .collect();

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}
