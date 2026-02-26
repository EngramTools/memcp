//! MCP protocol contract tests via McpTestClient.
//!
//! Each test spawns a real memcp serve process with an isolated ephemeral
//! PostgreSQL database. Tests exercise each MCP tool operation end-to-end
//! over the JSON-RPC stdio protocol.
//!
//! These tests complement the existing integration_test.rs (which covers
//! store/get/delete). This file covers: list, reinforce, feedback, and recall.
//!
//! NOTE: McpTestClient is defined in integration_test.rs which is a sibling
//! integration test crate. Since Rust does not share code between integration
//! test crates, we redefine the minimal infrastructure here.

use std::process::{Command, Stdio};
use std::io::{Write, BufRead, BufReader};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use std::time::Duration;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// McpClient — synchronous stdio JSON-RPC client
// ---------------------------------------------------------------------------

struct McpClient {
    child: std::process::Child,
    tx: Sender<Value>,
    rx: Receiver<Value>,
}

impl McpClient {
    fn spawn_with_env(env_vars: Vec<(&str, String)>) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_memcp"));
        cmd.arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().expect("Failed to spawn memcp binary");
        let mut stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");

        let (req_tx, req_rx) = channel::<Value>();
        let (resp_tx, resp_rx) = channel::<Value>();

        thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                let s = serde_json::to_string(&request).expect("serialize");
                if writeln!(stdin, "{}", s).is_err() { break; }
                if stdin.flush().is_err() { break; }
            }
        });

        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if resp_tx.send(v).is_err() { break; }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        McpClient { child, tx: req_tx, rx: resp_rx }
    }

    fn send_request(&self, req: Value) -> Option<Value> {
        self.tx.send(req).ok()?;
        self.rx.recv_timeout(Duration::from_secs(15)).ok()
    }

    fn send_notification(&self, notif: Value) {
        let _ = self.tx.send(notif);
        thread::sleep(Duration::from_millis(50));
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// McpTestClient — wraps McpClient with ephemeral DB lifecycle
// ---------------------------------------------------------------------------

struct McpTestClient {
    inner: McpClient,
    db_name: String,
}

impl McpTestClient {
    async fn spawn() -> Self {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";
        let db_name = format!("memcp_contract_{}", uuid::Uuid::new_v4().simple());

        let base_pool = sqlx::PgPool::connect(base_url).await
            .expect("connect to base postgres for temp DB creation");

        sqlx::query(&format!("CREATE DATABASE {}", db_name))
            .execute(&base_pool).await
            .expect("CREATE DATABASE");

        let test_db_url = format!("postgres://memcp:memcp@localhost:5433/{}", db_name);
        let test_pool = sqlx::PgPool::connect(&test_db_url).await
            .expect("connect to temp DB");

        sqlx::migrate!("./migrations").run(&test_pool).await
            .expect("run migrations on temp DB");

        test_pool.close().await;
        base_pool.close().await;

        let inner = McpClient::spawn_with_env(vec![("DATABASE_URL", test_db_url)]);
        thread::sleep(Duration::from_millis(300));

        McpTestClient { inner, db_name }
    }

    async fn cleanup(self) {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";
        drop(self.inner);
        tokio::time::sleep(Duration::from_millis(100)).await;

        let base_pool = sqlx::PgPool::connect(base_url).await
            .expect("connect to base postgres for cleanup");

        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db_name))
            .execute(&base_pool).await
            .expect("DROP DATABASE");

        base_pool.close().await;
    }

    fn send_request(&self, req: Value) -> Option<Value> {
        self.inner.send_request(req)
    }

    fn send_notification(&self, notif: Value) {
        self.inner.send_notification(notif);
    }
}

// ---------------------------------------------------------------------------
// Handshake helper
// ---------------------------------------------------------------------------

fn handshake(client: &McpTestClient) {
    client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mcp-contract-test", "version": "1.0"}
        }
    })).expect("init failed");

    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));
}

/// Store a memory and return its ID.
fn store_memory(client: &McpTestClient, content: &str, id_counter: i64) -> String {
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": id_counter,
        "params": {
            "name": "store_memory",
            "arguments": {
                "content": content,
                "type_hint": "fact",
                "source": "mcp-contract-test"
            }
        }
    })).expect("store_memory failed");

    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "store_memory should succeed: {}",
        resp
    );

    resp["result"]["structuredContent"]["id"]
        .as_str()
        .expect("store_memory should return structuredContent.id")
        .to_string()
}

// ---------------------------------------------------------------------------
// Test 1: store_memory — returns ID, status success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_store_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 2,
        "params": {
            "name": "store_memory",
            "arguments": {
                "content": "PostgreSQL is the primary database for this project",
                "type_hint": "fact",
                "source": "mcp-contract-test",
                "tags": ["database", "infrastructure"]
            }
        }
    })).expect("store_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "store_memory should not return isError: {}",
        resp
    );

    let sc = &resp["result"]["structuredContent"];
    assert!(sc["id"].is_string(), "should have string id: {}", sc);
    assert_eq!(sc["content"], "PostgreSQL is the primary database for this project");
    assert_eq!(sc["type_hint"], "fact");
    assert_eq!(sc["source"], "mcp-contract-test");
    assert!(sc["created_at"].is_string(), "should have created_at timestamp");

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 2: search_memory — BM25 finds stored memory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_search_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let memory_id = store_memory(&client, "Rust async programming with Tokio runtime", 2);

    // Search with matching terms
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "search_memory",
            "arguments": {
                "query": "Rust async Tokio"
            }
        }
    })).expect("search_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "search_memory should succeed: {}",
        resp
    );

    let sc = &resp["result"]["structuredContent"];
    let memories = sc["memories"].as_array()
        .expect("search result should have memories array");

    let found = memories.iter().any(|m| m["id"].as_str() == Some(&memory_id));
    assert!(
        found,
        "stored memory {} should appear in search results: {}",
        memory_id, sc
    );

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 3: list_memories — returns stored memories
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_list_memories() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    // Store 3 memories
    for i in 0..3 {
        store_memory(&client, &format!("List contract test memory number {}", i), (2 + i) as i64);
    }

    // List memories
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 10,
        "params": {
            "name": "list_memories",
            "arguments": {
                "limit": 20
            }
        }
    })).expect("list_memories request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "list_memories should succeed: {}",
        resp
    );

    let sc = &resp["result"]["structuredContent"];
    let memories = sc["memories"].as_array()
        .expect("list result should have memories array");

    assert!(
        memories.len() >= 3,
        "list should return at least 3 memories, got {}: {}",
        memories.len(), sc
    );

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 4: get_memory — returns full memory object
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_get_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let memory_id = store_memory(&client, "Dark mode preference for all IDEs and terminals", 2);

    // Get the memory back
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "get_memory",
            "arguments": {
                "id": memory_id.clone()
            }
        }
    })).expect("get_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "get_memory should succeed: {}",
        resp
    );

    let memory = &resp["result"]["structuredContent"];
    assert_eq!(memory["id"], memory_id, "id should match stored id");
    assert_eq!(
        memory["content"], "Dark mode preference for all IDEs and terminals",
        "content should round-trip"
    );
    assert_eq!(memory["type_hint"], "fact");
    assert_eq!(memory["source"], "mcp-contract-test");
    assert!(memory["created_at"].is_string(), "should have created_at");

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 5: delete_memory — deletes; subsequent get returns error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_delete_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let memory_id = store_memory(&client, "Memory to be deleted via MCP contract test", 2);

    // Delete the memory
    let delete_resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "delete_memory",
            "arguments": {
                "id": memory_id.clone()
            }
        }
    })).expect("delete_memory request failed");

    assert!(
        delete_resp["result"]["isError"].is_null() || delete_resp["result"]["isError"] == false,
        "delete_memory should succeed: {}",
        delete_resp
    );

    // Attempt get — should return isError: true
    let get_resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 4,
        "params": {
            "name": "get_memory",
            "arguments": {
                "id": memory_id
            }
        }
    })).expect("get_memory request failed");

    assert_eq!(
        get_resp["result"]["isError"], true,
        "get_memory after delete should return isError: true: {}",
        get_resp
    );

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 6: reinforce_memory — returns success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_reinforce_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let memory_id = store_memory(&client, "Memory for reinforce contract test", 2);

    // Call reinforce_memory
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "reinforce_memory",
            "arguments": {
                "id": memory_id,
                "quality": "good"
            }
        }
    })).expect("reinforce_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "reinforce_memory should succeed: {}",
        resp
    );

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 7: feedback_memory — returns success for "useful" signal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_feedback_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    let memory_id = store_memory(&client, "Memory for feedback contract test", 2);

    // Call feedback_memory with signal="useful"
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "feedback_memory",
            "arguments": {
                "id": memory_id,
                "signal": "useful"
            }
        }
    })).expect("feedback_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "feedback_memory with 'useful' signal should succeed: {}",
        resp
    );

    // Also test "irrelevant" signal
    let memory_id2 = store_memory(&client, "Memory for irrelevant feedback test", 4);

    let resp2 = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 5,
        "params": {
            "name": "feedback_memory",
            "arguments": {
                "id": memory_id2,
                "signal": "irrelevant"
            }
        }
    })).expect("feedback_memory irrelevant request failed");

    assert!(
        resp2["result"]["isError"].is_null() || resp2["result"]["isError"] == false,
        "feedback_memory with 'irrelevant' signal should succeed: {}",
        resp2
    );

    client.cleanup().await;
}

// ---------------------------------------------------------------------------
// Test 8: recall_memory — returns memories for matching query
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mcp_recall_memory() {
    let client = McpTestClient::spawn().await;
    handshake(&client);

    // Store a memory with known distinctive content
    store_memory(&client, "Rust ownership and borrowing are core language features", 2);

    // Call recall_memory — relies on BM25 (no daemon for vector embeddings)
    let resp = client.send_request(json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 3,
        "params": {
            "name": "recall_memory",
            "arguments": {
                "query": "Rust ownership borrowing",
                "session_id": "contract-test-session-recall"
            }
        }
    })).expect("recall_memory request failed");

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 3);

    // recall_memory should not return a protocol-level error
    // It may return an empty result (no vector embeddings in serve mode) or actual results
    // Either way, isError should be false
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "recall_memory should not return isError (empty result is OK): {}",
        resp
    );

    client.cleanup().await;
}
