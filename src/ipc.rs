//! IPC channel between CLI and daemon for embedding queries.
//!
//! The daemon holds the loaded fastembed model (87MB, too heavy to reload per CLI invocation).
//! This module provides:
//!   - `embed_socket_path()` — well-known Unix domain socket path
//!   - `start_embed_listener()` — daemon-side: binds socket, serves embed requests
//!   - `embed_via_daemon()` — CLI-side: connects, sends text, receives embedding vector
//!
//! Protocol: newline-delimited JSON over a Unix domain socket.
//!   Request:  {"text": "query text"}
//!   Response: {"embedding": [0.1, 0.2, ...]}  or  {"error": "message"}
//!
//! Design: fail-open. CLI caller receives `None` on any error (timeout, connection
//! refused, socket absent, parse error) and falls back to text-only search with
//! a stderr warning. This matches the fail-open pattern used throughout memcp.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::embedding::EmbeddingProvider;

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

/// Spawn the embed IPC listener as a background task.
///
/// Called by `run_daemon()` alongside existing worker spawns. The listener
/// accepts incoming connections, each handled in an independent tokio task.
///
/// **Stale socket handling (Pitfall 5):** Before binding, we attempt a connect.
/// If connection is refused, the socket is stale — remove it and re-bind.
/// This is standard Unix daemon practice.
pub async fn start_embed_listener(
    socket_path: PathBuf,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
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
        tokio::spawn(async move {
            if let Err(e) = handle_embed_connection(stream, provider).await {
                tracing::debug!(error = %e, "Embed IPC connection error");
            }
        });
    }
}

/// Handle a single embed IPC connection.
///
/// Reads one JSON line, embeds the text, writes one JSON line back.
async fn handle_embed_connection(
    mut stream: UnixStream,
    provider: Arc<dyn EmbeddingProvider + Send + Sync>,
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
    let text = request["text"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'text' field in embed request"))?;

    let response = match provider.embed(text).await {
        Ok(embedding) => serde_json::json!({ "embedding": embedding }),
        Err(e) => {
            tracing::warn!(error = %e, "Embedding failed in IPC handler");
            serde_json::json!({ "error": e.to_string() })
        }
    };

    let mut response_line = serde_json::to_string(&response)?;
    response_line.push('\n');
    write_half.write_all(response_line.as_bytes()).await?;
    write_half.flush().await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI-side client
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
