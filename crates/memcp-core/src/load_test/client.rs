//! Concurrent HTTP client driver for load test workloads.
//!
//! Spawns `config.total_ops` async tasks bounded by a `Semaphore` with
//! `config.concurrency` permits. Each task sends one HTTP request to the
//! test server, measures round-trip latency, and records a `RequestResult`.
//!
//! Read/write distribution follows `config.rw_ratio.write_pct()`:
//! - Writes: store, update, annotate, delete (cycled via op index)
//! - Reads:  search, recall, discover, export (cycled via op index)
//!
//! Stored IDs are tracked in a shared `Arc<Mutex<Vec<String>>>` so that
//! update, annotate, and delete operations can reference real IDs.

use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;

use super::{LoadTestConfig};
use super::metrics::RequestResult;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Run the full concurrent workload against the test HTTP server.
///
/// Spawns `config.total_ops` tasks, bounded to `config.concurrency` concurrent
/// requests via a `Semaphore`. Each task sends one HTTP request, records the
/// `RequestResult`, and drops its permit.
///
/// Returns all results after every task has completed (or timed out / errored).
/// Individual request failures are recorded as `is_error: true` — they do NOT
/// propagate as Rust errors.
pub async fn run_workload(
    config: &LoadTestConfig,
    client: &reqwest::Client,
) -> Vec<RequestResult> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let results: Arc<Mutex<Vec<RequestResult>>> = Arc::new(Mutex::new(Vec::with_capacity(config.total_ops)));
    // Track IDs returned by /v1/store so update/annotate/delete can use them
    let stored_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Progress bar
    let pb = ProgressBar::new(config.total_ops as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} ops ({per_sec}, eta {eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );
    let pb = Arc::new(pb);

    let write_pct = config.rw_ratio.write_pct();
    let base_url = config.base_url.clone();

    let mut handles = Vec::with_capacity(config.total_ops);

    for op_index in 0..config.total_ops {
        let permit = semaphore.clone().acquire_owned().await.expect("semaphore acquire");
        let client = client.clone();
        let results = results.clone();
        let stored_ids = stored_ids.clone();
        let base_url = base_url.clone();
        let pb = pb.clone();

        let is_write = (op_index % 100) < write_pct;

        let handle = tokio::spawn(async move {
            let result = if is_write {
                run_write_op(&client, &base_url, op_index, &stored_ids).await
            } else {
                run_read_op(&client, &base_url, op_index).await
            };

            results.lock().await.push(result);
            pb.inc(1);

            // Permit drops here, freeing one slot in the semaphore
            drop(permit);
        });

        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        // Ignore individual task panics — results are already recorded
        let _ = handle.await;
    }
    pb.finish_with_message("workload complete");

    Arc::try_unwrap(results)
        .expect("all tasks complete, no other Arc holders")
        .into_inner()
}

// ─── Write Operations ─────────────────────────────────────────────────────────

/// Execute one write operation, cycling through store → update → annotate → delete.
///
/// Write cycle (op_index % 4):
///   0 → POST /v1/store     — always available
///   1 → POST /v1/update    — skip if no stored IDs
///   2 → POST /v1/annotate  — skip if no stored IDs
///   3 → DELETE /v1/memories/{id} — skip if no stored IDs
///
/// Skipped operations fall back to POST /v1/store.
async fn run_write_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
    stored_ids: &Arc<Mutex<Vec<String>>>,
) -> RequestResult {
    match op_index % 4 {
        0 => store_op(client, base_url, op_index, stored_ids).await,
        1 => {
            let id = pick_id(stored_ids).await;
            if let Some(id) = id {
                update_op(client, base_url, op_index, &id).await
            } else {
                store_op(client, base_url, op_index, stored_ids).await
            }
        }
        2 => {
            let id = pick_id(stored_ids).await;
            if let Some(id) = id {
                annotate_op(client, base_url, op_index, &id).await
            } else {
                store_op(client, base_url, op_index, stored_ids).await
            }
        }
        3 => {
            let id = pop_id(stored_ids).await;
            if let Some(id) = id {
                delete_op(client, base_url, &id).await
            } else {
                store_op(client, base_url, op_index, stored_ids).await
            }
        }
        _ => unreachable!(),
    }
}

/// Execute one read operation, cycling through search → recall → discover → export.
///
/// Read cycle (op_index % 4):
///   0 → POST /v1/search   — 50 unique queries for cache variation
///   1 → POST /v1/recall   — 50 unique queries
///   2 → POST /v1/discover — fixed pattern discovery query
///   3 → GET  /v1/export   — export all (may return large payload)
async fn run_read_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
) -> RequestResult {
    match op_index % 4 {
        0 => search_op(client, base_url, op_index).await,
        1 => recall_op(client, base_url, op_index).await,
        2 => discover_op(client, base_url).await,
        3 => export_op(client, base_url).await,
        _ => unreachable!(),
    }
}

// ─── Individual Endpoint Operations ───────────────────────────────────────────

async fn store_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
    stored_ids: &Arc<Mutex<Vec<String>>>,
) -> RequestResult {
    let url = format!("{}/v1/store", base_url);
    let body = serde_json::json!({
        "content": format!("Load test memory {}", op_index),
        "type_hint": "fact",
        "tags": ["load-test"]
    });

    let start = Instant::now();
    match client.post(&url).json(&body).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency_ms = start.elapsed().as_millis() as u64;
            let is_error = status >= 400;

            // Parse and track the stored ID for subsequent operations
            if !is_error {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(id) = json.get("id").and_then(|v| v.as_str()) {
                        stored_ids.lock().await.push(id.to_string());
                    }
                }
            }

            RequestResult {
                endpoint: "/v1/store".to_string(),
                status,
                latency_ms,
                is_error,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            tracing::debug!(error = %e, "store request failed");
            RequestResult {
                endpoint: "/v1/store".to_string(),
                status: 0,
                latency_ms,
                is_error: true,
            }
        }
    }
}

async fn update_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
    id: &str,
) -> RequestResult {
    let url = format!("{}/v1/update", base_url);
    let body = serde_json::json!({
        "id": id,
        "content": format!("Updated load test memory {}", op_index)
    });

    timed_request(client, "POST", &url, Some(body), "/v1/update").await
}

async fn annotate_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
    id: &str,
) -> RequestResult {
    let url = format!("{}/v1/annotate", base_url);
    let body = serde_json::json!({
        "id": id,
        "tags": ["annotated", format!("op-{}", op_index % 10)]
    });

    timed_request(client, "POST", &url, Some(body), "/v1/annotate").await
}

async fn delete_op(
    client: &reqwest::Client,
    base_url: &str,
    id: &str,
) -> RequestResult {
    let url = format!("{}/v1/memories/{}", base_url, id);

    let start = Instant::now();
    match client.delete(&url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency_ms = start.elapsed().as_millis() as u64;
            RequestResult {
                endpoint: "/v1/delete".to_string(),
                status,
                latency_ms,
                is_error: status >= 400,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            tracing::debug!(error = %e, "delete request failed");
            RequestResult {
                endpoint: "/v1/delete".to_string(),
                status: 0,
                latency_ms,
                is_error: true,
            }
        }
    }
}

async fn search_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
) -> RequestResult {
    let url = format!("{}/v1/search", base_url);
    // 50 unique queries for cache variation
    let body = serde_json::json!({
        "query": format!("load test query {}", op_index % 50),
        "limit": 10
    });

    timed_request(client, "POST", &url, Some(body), "/v1/search").await
}

async fn recall_op(
    client: &reqwest::Client,
    base_url: &str,
    op_index: usize,
) -> RequestResult {
    let url = format!("{}/v1/recall", base_url);
    // Use first=true for queryless recall (avoids need for embed_provider)
    let body = serde_json::json!({
        "first": true,
        "limit": 5
    });
    // Suppress unused op_index warning in recall
    let _ = op_index;

    timed_request(client, "POST", &url, Some(body), "/v1/recall").await
}

async fn discover_op(
    client: &reqwest::Client,
    base_url: &str,
) -> RequestResult {
    let url = format!("{}/v1/discover", base_url);
    let body = serde_json::json!({
        "query": "discover patterns",
        "limit": 5
    });

    timed_request(client, "POST", &url, Some(body), "/v1/discover").await
}

async fn export_op(
    client: &reqwest::Client,
    base_url: &str,
) -> RequestResult {
    let url = format!("{}/v1/export", base_url);

    let start = Instant::now();
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // Consume the body to get accurate end-to-end latency
            let _ = resp.bytes().await;
            let latency_ms = start.elapsed().as_millis() as u64;
            RequestResult {
                endpoint: "/v1/export".to_string(),
                status,
                latency_ms,
                is_error: status >= 400,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            tracing::debug!(error = %e, "export request failed");
            RequestResult {
                endpoint: "/v1/export".to_string(),
                status: 0,
                latency_ms,
                is_error: true,
            }
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Generic timed POST request helper. Records latency from send to response.
async fn timed_request(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    body: Option<serde_json::Value>,
    endpoint_label: &str,
) -> RequestResult {
    let start = Instant::now();

    let req = match method {
        "POST" => client.post(url),
        "GET" => client.get(url),
        _ => client.post(url),
    };

    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };

    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // Consume body for accurate latency
            let _ = resp.bytes().await;
            let latency_ms = start.elapsed().as_millis() as u64;
            RequestResult {
                endpoint: endpoint_label.to_string(),
                status,
                latency_ms,
                is_error: status >= 400,
            }
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            tracing::debug!(error = %e, endpoint = endpoint_label, "request failed");
            RequestResult {
                endpoint: endpoint_label.to_string(),
                status: 0,
                latency_ms,
                is_error: true,
            }
        }
    }
}

/// Pick a random ID from the stored IDs pool without removing it.
/// Returns `None` if the pool is empty.
async fn pick_id(stored_ids: &Arc<Mutex<Vec<String>>>) -> Option<String> {
    let ids = stored_ids.lock().await;
    if ids.is_empty() {
        None
    } else {
        // Pick from the middle to avoid hot-spotting the newest entries
        let idx = ids.len() / 2;
        Some(ids[idx].clone())
    }
}

/// Pop the last ID from the stored IDs pool (for delete operations).
/// Returns `None` if the pool is empty.
async fn pop_id(stored_ids: &Arc<Mutex<Vec<String>>>) -> Option<String> {
    stored_ids.lock().await.pop()
}
