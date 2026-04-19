use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// Locate the memcp binary in the workspace target directory.
/// In a workspace, binaries from other crates are built into the shared target/ dir.
fn memcp_bin_path() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go up from crates/memcp-core to workspace root
    path.pop(); // crates/
    path.pop(); // workspace root
    path.push("target");
    path.push("debug");
    path.push("memcp");
    path
}

/// Helper struct to manage server process with async I/O
struct McpClient {
    child: std::process::Child,
    tx: Sender<Value>,
    rx: Receiver<Value>,
}

impl McpClient {
    fn spawn() -> Self {
        Self::spawn_with_env(vec![])
    }

    fn spawn_with_env(env_vars: Vec<(&str, String)>) -> Self {
        let mut cmd = Command::new(memcp_bin_path());
        cmd.arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()); // Suppress log output in tests

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().expect("Failed to spawn memcp binary");

        let mut stdin = child.stdin.take().expect("Failed to get stdin");
        let stdout = child.stdout.take().expect("Failed to get stdout");

        // Channel for sending requests
        let (req_tx, req_rx) = channel::<Value>();

        // Channel for receiving responses
        let (resp_tx, resp_rx) = channel::<Value>();

        // Thread to write requests to stdin
        thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                let request_str = serde_json::to_string(&request).expect("Failed to serialize");
                if writeln!(stdin, "{}", request_str).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        // Thread to read responses from stdout
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&line) {
                            if resp_tx.send(value).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        McpClient {
            child,
            tx: req_tx,
            rx: resp_rx,
        }
    }

    fn send_request(&self, request: Value) -> Option<Value> {
        self.tx.send(request).ok()?;
        self.rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    fn send_notification(&self, notification: Value) {
        let _ = self.tx.send(notification);
        // Notifications don't have responses, give server time to process
        thread::sleep(Duration::from_millis(50));
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// =============================================================================
// McpTestClient — wraps McpClient with temp DB lifecycle management
// =============================================================================

/// Wraps McpClient and manages an ephemeral PostgreSQL database.
///
/// Each McpTestClient creates its own temp DB before spawning the child process
/// and drops the DB on cleanup(). This isolates MCP protocol tests from the dev
/// database and from each other.
struct McpTestClient {
    inner: McpClient,
    db_name: String,
}

impl McpTestClient {
    /// Spawn a McpTestClient with its own isolated temp database.
    ///
    /// Steps:
    /// 1. Connect to base postgres DB (maintenance connection)
    /// 2. CREATE DATABASE memcp_test_<uuid>
    /// 3. Run migrations on the new DB
    /// 4. Spawn McpClient child with DATABASE_URL pointing at the temp DB
    async fn spawn() -> Self {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";
        let db_name = format!("memcp_test_{}", uuid::Uuid::new_v4().simple());

        // Connect to base postgres DB for admin operations
        let base_pool = sqlx::PgPool::connect(base_url)
            .await
            .expect("Failed to connect to base postgres DB for temp DB creation");

        // Create the temp DB
        sqlx::query(&format!("CREATE DATABASE {}", db_name))
            .execute(&base_pool)
            .await
            .expect("Failed to CREATE DATABASE");

        // Connect to the new temp DB and run migrations
        let test_db_url = format!("postgres://memcp:memcp@localhost:5433/{}", db_name);
        let test_pool = sqlx::PgPool::connect(&test_db_url)
            .await
            .expect("Failed to connect to temp DB");

        sqlx::migrate!("./migrations")
            .run(&test_pool)
            .await
            .expect("Failed to run migrations on temp DB");

        // Close both pools before spawning child (child will open its own connections)
        test_pool.close().await;
        base_pool.close().await;

        // Spawn McpClient child with DATABASE_URL pointing at the temp DB
        let inner = McpClient::spawn_with_env(vec![("DATABASE_URL", test_db_url)]);

        // Give the child a moment to initialize
        thread::sleep(Duration::from_millis(200));

        McpTestClient { inner, db_name }
    }

    /// Drop the temp database. Call at the end of each test.
    ///
    /// Uses DROP DATABASE ... WITH (FORCE) to terminate any active connections
    /// before dropping (requires Postgres 13+).
    async fn cleanup(self) {
        let base_url = "postgres://memcp:memcp@localhost:5433/postgres";

        // Kill the child process first so it releases its connections
        drop(self.inner);

        // Brief wait for connections to close
        tokio::time::sleep(Duration::from_millis(100)).await;

        let base_pool = sqlx::PgPool::connect(base_url)
            .await
            .expect("Failed to connect to base postgres DB for cleanup");

        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db_name))
            .execute(&base_pool)
            .await
            .expect("Failed to DROP DATABASE");

        base_pool.close().await;
    }

    fn send_request(&self, request: Value) -> Option<Value> {
        self.inner.send_request(request)
    }

    fn send_notification(&self, notification: Value) {
        self.inner.send_notification(notification);
    }
}

// =============================================================================
// Shared helpers
// =============================================================================

/// Send the initialize + initialized handshake. Required before calling any tools.
#[allow(dead_code)]
fn initialize_mcp_client(client: &McpClient) {
    let init = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client.send_request(init).expect("init failed");
    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));
}

/// Same handshake for McpTestClient.
fn initialize_mcp_test_client(client: &McpTestClient) {
    let init = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client.send_request(init).expect("init failed");
    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));
}

// =============================================================================
// MCP Protocol Tests (require a running PostgreSQL instance)
// =============================================================================

#[test]
fn test_initialize_handshake() {
    let client = McpClient::spawn();

    // Send initialize request
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        }
    });

    let response = client
        .send_request(initialize_request)
        .expect("Failed to get initialize response");

    // Verify response structure
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["result"].is_object());

    let result = &response["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"]["tools"].is_object());
    assert_eq!(result["serverInfo"]["name"], "memcp");
    assert!(result["serverInfo"]["version"].is_string());
    assert!(result["serverInfo"]["description"].is_string());

    // Send initialized notification (no response expected)
    let initialized_notification = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    client.send_notification(initialized_notification);
}

#[test]
fn test_tool_discovery() {
    let client = McpClient::spawn();

    // Initialize first
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client
        .send_request(initialize_request)
        .expect("Failed to initialize");

    // Send initialized notification
    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // Send tools/list request
    let tools_list_request = json!({
        "jsonrpc": "2.0",
        "method": "tools/list",
        "id": 2
    });

    let response = client
        .send_request(tools_list_request)
        .expect("Failed to get tools/list response");

    // Verify response
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"]["tools"].is_array());

    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 18, "Should have exactly 18 tools (Phase 24.5 added ingest_messages + ingest_message)");

    // Check all expected tools are present
    let tool_names: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    assert!(tool_names.contains(&"store_memory".to_string()));
    assert!(tool_names.contains(&"get_memory".to_string()));
    assert!(tool_names.contains(&"update_memory".to_string()));
    assert!(tool_names.contains(&"delete_memory".to_string()));
    assert!(tool_names.contains(&"bulk_delete_memories".to_string()));
    assert!(tool_names.contains(&"list_memories".to_string()));
    assert!(tool_names.contains(&"search_memory".to_string()));
    assert!(tool_names.contains(&"reinforce_memory".to_string()));
    assert!(tool_names.contains(&"health_check".to_string()));
    assert!(tool_names.contains(&"feedback_memory".to_string()));
    assert!(tool_names.contains(&"recall_memory".to_string()));
    assert!(tool_names.contains(&"annotate_memory".to_string()));
    assert!(tool_names.contains(&"discover_memories".to_string()));
    assert!(
        tool_names.contains(&"ingest_messages".to_string()),
        "expected ingest_messages tool (Phase 24.5)"
    );
    assert!(
        tool_names.contains(&"ingest_message".to_string()),
        "expected ingest_message tool (Phase 24.5)"
    );

    // Verify each tool has required fields
    for tool in tools {
        assert!(tool["name"].is_string());
        assert!(tool["description"].is_string());
        assert!(tool["inputSchema"].is_object());
    }
}

#[test]
fn test_store_memory_success() {
    let client = McpClient::spawn();

    // Give PostgreSQL store a moment to initialize before the first request
    thread::sleep(Duration::from_millis(200));

    // Initialize
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client
        .send_request(initialize_request)
        .expect("Failed to initialize");

    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // Call store_memory tool with valid params (Phase 2 API: content, type_hint, source, tags)
    let store_request = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 2,
        "params": {
            "name": "store_memory",
            "arguments": {
                "content": "test memory content",
                "type_hint": "fact",
                "source": "test"
            }
        }
    });

    let response = client
        .send_request(store_request)
        .expect("Failed to get store_memory response");

    // Verify success response
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"].is_object());

    let result = &response["result"];
    assert!(result["isError"].is_null() || result["isError"] == false);

    // Check structured content
    assert!(result["content"].is_array());

    // Check for structuredContent (rmcp 0.15 uses this field)
    if result["structuredContent"].is_object() {
        let content = &result["structuredContent"];
        assert!(content["id"].is_string(), "Should have an ID");
        assert_eq!(content["content"], "test memory content");
        assert_eq!(content["type_hint"], "fact");
        assert_eq!(content["source"], "test");
        assert!(content["created_at"].is_string(), "Should have timestamp");

        // Verify ID looks like a UUID
        let id_str = content["id"].as_str().unwrap();
        assert!(id_str.contains('-'), "ID should be UUID-like");
    }
}

#[test]
fn test_store_memory_validation_error() {
    let client = McpClient::spawn();

    // Initialize
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client
        .send_request(initialize_request)
        .expect("Failed to initialize");

    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // Call store_memory with empty content (should fail validation)
    let store_request = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 2,
        "params": {
            "name": "store_memory",
            "arguments": {
                "content": ""
            }
        }
    });

    let response = client
        .send_request(store_request)
        .expect("Failed to get store_memory response");

    // Verify validation error response
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"].is_object());

    let result = &response["result"];
    assert_eq!(result["isError"], true, "Should have isError: true");

    // Check error message mentions "content"
    let content_arr = result["content"]
        .as_array()
        .expect("content should be array");
    let error_text = content_arr[0]["text"]
        .as_str()
        .expect("should have error text");
    assert!(
        error_text.to_lowercase().contains("content"),
        "Error message should mention 'content': {}",
        error_text
    );
}

#[test]
fn test_health_check() {
    let client = McpClient::spawn();

    // Initialize
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client
        .send_request(initialize_request)
        .expect("Failed to initialize");

    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // Call health_check tool
    let health_request = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 2,
        "params": {
            "name": "health_check",
            "arguments": {}
        }
    });

    let response = client
        .send_request(health_request)
        .expect("Failed to get health_check response");

    // Verify health check response
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"].is_object());

    let result = &response["result"];
    assert!(result["isError"].is_null() || result["isError"] == false);

    // Check structured content for health data
    if result["structuredContent"].is_object() {
        let health = &result["structuredContent"];
        assert_eq!(health["status"], "ok");
        assert!(health["version"].is_string());
        assert!(health["uptime_seconds"].is_number());
    }
}

#[test]
fn test_search_memory() {
    let client = McpClient::spawn();

    // Initialize
    let initialize_request = json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "id": 1,
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"}
        }
    });
    client
        .send_request(initialize_request)
        .expect("Failed to initialize");

    client.send_notification(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    // Call search_memory tool
    let search_request = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "id": 2,
        "params": {
            "name": "search_memory",
            "arguments": {
                "query": "test"
            }
        }
    });

    let response = client
        .send_request(search_request)
        .expect("Failed to get search_memory response");

    // Verify search response
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 2);
    assert!(response["result"].is_object());

    let result = &response["result"];
    assert!(result["isError"].is_null() || result["isError"] == false);

    // Check structured content for search results
    if result["structuredContent"].is_object() {
        let search_result = &result["structuredContent"];
        // Server returns "memories" array (not "results")
        assert!(
            search_result["memories"].is_array() || search_result["total_results"].is_number(),
            "Should have memories array or total_results field"
        );

        if let Some(memories) = search_result["memories"].as_array() {
            if !memories.is_empty() {
                let first_result = &memories[0];
                assert!(first_result["id"].is_string());
                assert!(first_result["content"].is_string());
                assert!(first_result["relevance_score"].is_number());
                assert!(first_result["created_at"].is_string());
            }
        }
    }
}

// =============================================================================
// Phase 07.2 Plan 04: MCP Protocol Contract Tests
// These tests use McpTestClient so each gets its own isolated temp database.
// =============================================================================

/// Contract test: store via JSON-RPC then get back and verify data persists end-to-end.
///
/// This test proves more than response format — it verifies the MCP layer
/// actually writes to and reads from the database correctly.
#[tokio::test]
async fn test_store_then_get_contract() {
    let client = McpTestClient::spawn().await;

    // Initialize MCP handshake
    initialize_mcp_test_client(&client);

    // Store a memory via JSON-RPC
    let store_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 2,
            "params": {
                "name": "store_memory",
                "arguments": {
                    "content": "contract test data",
                    "type_hint": "fact",
                    "source": "contract-test"
                }
            }
        }))
        .expect("Failed to get store_memory response");

    assert_eq!(store_response["jsonrpc"], "2.0");
    assert_eq!(store_response["id"], 2);
    assert!(
        store_response["result"]["isError"].is_null()
            || store_response["result"]["isError"] == false,
        "store_memory should not return isError: {}",
        store_response
    );

    // Extract the memory ID from structuredContent
    let stored_id = store_response["result"]["structuredContent"]["id"]
        .as_str()
        .expect("store_memory should return structuredContent.id");
    let stored_id = stored_id.to_string();

    // Get the memory back via JSON-RPC
    let get_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 3,
            "params": {
                "name": "get_memory",
                "arguments": {
                    "id": stored_id
                }
            }
        }))
        .expect("Failed to get get_memory response");

    assert_eq!(get_response["jsonrpc"], "2.0");
    assert_eq!(get_response["id"], 3);

    let get_result = &get_response["result"];
    assert!(
        get_result["isError"].is_null() || get_result["isError"] == false,
        "get_memory should not return isError: {}",
        get_response
    );

    // Verify data persisted correctly
    let memory = &get_result["structuredContent"];
    assert_eq!(
        memory["content"], "contract test data",
        "content should round-trip through store→get: {}",
        memory
    );
    assert_eq!(
        memory["type_hint"], "fact",
        "type_hint should round-trip through store→get: {}",
        memory
    );
    assert_eq!(
        memory["source"], "contract-test",
        "source should round-trip through store→get: {}",
        memory
    );

    client.cleanup().await;
}

/// Contract test: store via JSON-RPC then search and verify result appears in search.
///
/// Tests the full MCP store-to-search pipeline including BM25 indexing.
#[tokio::test]
async fn test_store_then_search_contract() {
    let client = McpTestClient::spawn().await;

    initialize_mcp_test_client(&client);

    // Store a memory with distinctive content for searching
    let store_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 2,
            "params": {
                "name": "store_memory",
                "arguments": {
                    "content": "Rust memory persistence verification",
                    "type_hint": "fact",
                    "source": "contract-test"
                }
            }
        }))
        .expect("Failed to store memory");

    assert!(
        store_response["result"]["isError"].is_null()
            || store_response["result"]["isError"] == false,
        "store_memory should succeed: {}",
        store_response
    );

    // Extract the stored ID
    let stored_id = store_response["result"]["structuredContent"]["id"]
        .as_str()
        .expect("store should return structuredContent.id")
        .to_string();

    // Search for the stored memory
    let search_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 3,
            "params": {
                "name": "search_memory",
                "arguments": {
                    "query": "Rust memory persistence"
                }
            }
        }))
        .expect("Failed to search memory");

    assert_eq!(search_response["jsonrpc"], "2.0");
    assert_eq!(search_response["id"], 3);
    assert!(
        search_response["result"]["isError"].is_null()
            || search_response["result"]["isError"] == false,
        "search_memory should not return isError: {}",
        search_response
    );

    // Verify the stored memory appears in search results
    let search_result = &search_response["result"]["structuredContent"];
    let memories = search_result["memories"]
        .as_array()
        .expect("search_memory should return memories array");

    let found = memories
        .iter()
        .any(|m| m["id"].as_str() == Some(&stored_id));
    assert!(
        found,
        "Stored memory (id={}) should appear in search results. Got: {}",
        stored_id, search_result
    );

    client.cleanup().await;
}

/// Contract test: store via JSON-RPC, delete, then get should return error.
///
/// Tests the full MCP store-delete-get lifecycle to verify deletion actually removes data.
#[tokio::test]
async fn test_store_then_delete_contract() {
    let client = McpTestClient::spawn().await;

    initialize_mcp_test_client(&client);

    // Store a memory
    let store_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 2,
            "params": {
                "name": "store_memory",
                "arguments": {
                    "content": "memory to be deleted",
                    "type_hint": "fact",
                    "source": "contract-test"
                }
            }
        }))
        .expect("Failed to store memory");

    assert!(
        store_response["result"]["isError"].is_null()
            || store_response["result"]["isError"] == false,
        "store_memory should succeed: {}",
        store_response
    );

    let stored_id = store_response["result"]["structuredContent"]["id"]
        .as_str()
        .expect("store should return structuredContent.id")
        .to_string();

    // Delete the memory
    let delete_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 3,
            "params": {
                "name": "delete_memory",
                "arguments": {
                    "id": stored_id.clone()
                }
            }
        }))
        .expect("Failed to delete memory");

    assert_eq!(delete_response["jsonrpc"], "2.0");
    assert_eq!(delete_response["id"], 3);
    assert!(
        delete_response["result"]["isError"].is_null()
            || delete_response["result"]["isError"] == false,
        "delete_memory should succeed: {}",
        delete_response
    );

    // Attempt to get the deleted memory — should return isError: true or empty
    let get_response = client
        .send_request(json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "id": 4,
            "params": {
                "name": "get_memory",
                "arguments": {
                    "id": stored_id
                }
            }
        }))
        .expect("Failed to send get_memory request");

    assert_eq!(get_response["jsonrpc"], "2.0");
    assert_eq!(get_response["id"], 4);

    // After deletion, get_memory should return isError: true
    let get_result = &get_response["result"];
    assert_eq!(
        get_result["isError"], true,
        "get_memory after delete should return isError: true. Got: {}",
        get_response
    );

    client.cleanup().await;
}

// =============================================================================
// Phase 07.5 Wave 0 Test Stubs
// =============================================================================

/// SCF-01: CLI search degrades gracefully when daemon is offline.
///
/// Contract: `memcp search "query"` with no daemon running must:
///   - exit with code 0 (graceful degradation, not crash)
///   - emit a warning to stderr about degraded/daemon/text-only mode (added in Plan 01)
///   - emit valid JSON to stdout (even if results are empty)
///
/// This test uses std::process::Command to spawn the real binary with DATABASE_URL
/// set so the store connects, but the daemon embed socket is intentionally absent.
///
/// Note: Plan 01 will add the --json flag and stderr degradation warning.
/// This stub tests the subset of that contract that can be verified today:
/// exit 0 + valid JSON stdout. Plan 01 must additionally assert the stderr warning.
#[test]
fn test_cli_search_daemon_offline() {
    // DATABASE_URL must be set for the store to connect (tests require Postgres).
    // If DATABASE_URL is not set, skip this test gracefully.
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("test_cli_search_daemon_offline: DATABASE_URL not set — skipping");
            return;
        }
    };

    // Run `memcp search "test query"` with no daemon running.
    // The daemon socket will not be present, so the CLI should fall back to
    // BM25-only search (current behavior) and warn on stderr (added in Plan 01).
    let output = std::process::Command::new(memcp_bin_path())
        .args(["search", "test query"])
        .env("DATABASE_URL", &database_url)
        .output()
        .expect("Failed to spawn memcp binary");

    // Must exit successfully (graceful degradation, not crash)
    assert!(
        output.status.success(),
        "memcp search should exit 0 when daemon is offline, got status: {}.\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // stdout must be valid JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "memcp search stdout must be valid JSON when daemon is offline.\nstdout: {}",
        stdout
    );

    // Plan 01 must additionally assert stderr contains a warning like:
    //   "daemon offline" / "degraded" / "text-only search"
    // That assertion is deferred here because the warning doesn't exist yet.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = stderr; // acknowledged — Plan 01 must add and assert the warning
}
