//! MCP server — rmcp-based tool handler for stdio transport.
//!
//! MemoryService implements ServerHandler with tools: store_memory, search_memory,
//! update_memory, delete_memory, list_memories, recall_memory, feedback_memory, etc.
//! Wires together storage/, intelligence/, and pipeline/ layers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// Session-scoped integer reference mapping for memory IDs.
///
/// LLMs frequently hallucinate plausible-looking UUIDs. By presenting sequential
/// integers (1, 2, 3...) in tool responses and accepting them as input, agents
/// avoid an entire class of invalid-ID errors.
///
/// One UuidRefMap per MemoryService instance (per MCP connection) — refs are
/// session-scoped and reset between connections.
pub struct UuidRefMap {
    ref_to_uuid: std::sync::Mutex<HashMap<u32, String>>,
    uuid_to_ref: std::sync::Mutex<HashMap<String, u32>>,
    next_ref: AtomicU32,
}

impl UuidRefMap {
    pub fn new() -> Self {
        Self {
            ref_to_uuid: std::sync::Mutex::new(HashMap::new()),
            uuid_to_ref: std::sync::Mutex::new(HashMap::new()),
            next_ref: AtomicU32::new(1), // Start from 1 (more natural for agents)
        }
    }

    /// Assign an integer ref to a UUID. Idempotent — same UUID always gets same ref.
    pub fn assign_ref(&self, uuid: &str) -> u32 {
        let mut uuid_map = self.uuid_to_ref.lock().unwrap();
        if let Some(&r) = uuid_map.get(uuid) {
            return r;
        }
        let r = self.next_ref.fetch_add(1, Ordering::SeqCst);
        uuid_map.insert(uuid.to_string(), r);
        self.ref_to_uuid.lock().unwrap().insert(r, uuid.to_string());
        r
    }

    /// Resolve an input string to a UUID. Accepts both integer refs and UUID strings.
    /// Returns None only if an integer ref is not found in the mapping.
    pub fn resolve(&self, input: &str) -> Option<String> {
        if let Ok(n) = input.parse::<u32>() {
            self.ref_to_uuid.lock().unwrap().get(&n).cloned()
        } else {
            Some(input.to_string()) // UUID passthrough
        }
    }
}

use rmcp::{
    ServerHandler,
    tool,
    model::{
        ServerCapabilities, Implementation, ProtocolVersion, CallToolResult,
        RawResource, ListResourcesResult, ReadResourceResult, ResourceContents,
        ReadResourceRequestParams, AnnotateAble, Meta,
    },
    handler::server::wrapper::Parameters,
    service::{RequestContext, RoleServer},
    ErrorData as McpError,
};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::DateTime;
use chrono::Utc;
use crate::query_intelligence::{RankedCandidate, temporal::parse_temporal_hint};
use crate::search::rrf_fuse_multi;

use crate::config::{IdempotencyConfig, RecallConfig, ResourceCapsConfig, SalienceConfig, SearchConfig};
use crate::content_filter::{ContentFilter, FilterVerdict};
use crate::embedding::{EmbeddingJob, EmbeddingProvider};
use crate::errors::MemcpError;
use crate::extraction::ExtractionJob;
use crate::search::{SalienceScorer, ScoredHit};
use crate::search::salience::{SalienceInput, dedup_parent_chunks};
use crate::store::{
    decode_search_keyset_cursor, encode_search_keyset_cursor,
    CreateMemory, ListFilter, Memory, MemoryStore, UpdateMemory,
};

pub struct MemoryService {
    store: Arc<dyn MemoryStore + Send + Sync>,
    pipeline: Option<crate::embedding::pipeline::EmbeddingPipeline>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pg_store: Option<Arc<crate::store::postgres::PostgresMemoryStore>>,
    salience_config: SalienceConfig,
    search_config: SearchConfig,
    start_time: Instant,
    extraction_pipeline: Option<crate::extraction::pipeline::ExtractionPipeline>,
    qi_expansion_provider: Option<Arc<dyn crate::query_intelligence::QueryIntelligenceProvider + Send + Sync>>,
    qi_reranking_provider: Option<Arc<dyn crate::query_intelligence::QueryIntelligenceProvider + Send + Sync>>,
    qi_config: crate::config::QueryIntelligenceConfig,
    content_filter: Option<Arc<dyn ContentFilter>>,
    idempotency_config: IdempotencyConfig,
    recall_config: RecallConfig,
    resource_caps: ResourceCapsConfig,
    extraction_enabled: bool,
    store_config: crate::config::StoreConfig,
    reembed_on_tag_change: bool,
    resource_limits: crate::config::ResourceLimitsConfig,
    gc_config: crate::config::GcConfig,
    last_auto_gc: Arc<std::sync::Mutex<Option<Instant>>>,
    ref_map: UuidRefMap,
}

impl MemoryService {
    pub fn new(
        store: Arc<dyn MemoryStore + Send + Sync>,
        pipeline: Option<crate::embedding::pipeline::EmbeddingPipeline>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
        pg_store: Option<Arc<crate::store::postgres::PostgresMemoryStore>>,
        salience_config: SalienceConfig,
        search_config: SearchConfig,
        extraction_pipeline: Option<crate::extraction::pipeline::ExtractionPipeline>,
        qi_expansion_provider: Option<Arc<dyn crate::query_intelligence::QueryIntelligenceProvider + Send + Sync>>,
        qi_reranking_provider: Option<Arc<dyn crate::query_intelligence::QueryIntelligenceProvider + Send + Sync>>,
        qi_config: crate::config::QueryIntelligenceConfig,
        content_filter: Option<Arc<dyn ContentFilter>>,
    ) -> Self {
        Self {
            store,
            pipeline,
            embedding_provider,
            pg_store,
            salience_config,
            search_config,
            start_time: Instant::now(),
            extraction_pipeline,
            qi_expansion_provider,
            qi_reranking_provider,
            qi_config,
            content_filter,
            idempotency_config: IdempotencyConfig::default(),
            recall_config: RecallConfig::default(),
            resource_caps: ResourceCapsConfig::default(),
            extraction_enabled: false,
            store_config: crate::config::StoreConfig::default(),
            reembed_on_tag_change: false,
            resource_limits: crate::config::ResourceLimitsConfig::default(),
            gc_config: crate::config::GcConfig::default(),
            last_auto_gc: Arc::new(std::sync::Mutex::new(None)),
            ref_map: UuidRefMap::new(),
        }
    }

    /// Update the recall configuration and extraction flag.
    ///
    /// Call after construction to wire config values from the full Config (e.g., in main.rs).
    pub fn set_recall_config(&mut self, config: RecallConfig, extraction_enabled: bool) {
        self.recall_config = config;
        self.extraction_enabled = extraction_enabled;
    }

    /// Update the idempotency configuration.
    ///
    /// Call after construction with the full Config values (e.g., from main.rs).
    pub fn set_idempotency_config(&mut self, config: IdempotencyConfig) {
        self.idempotency_config = config;
    }

    /// Update the resource caps configuration.
    ///
    /// Call after construction to wire container-level resource limits.
    /// Used by engram.host to enforce per-instance caps (max_memories, max_search_results).
    pub fn set_resource_caps(&mut self, config: ResourceCapsConfig) {
        self.resource_caps = config;
    }

    /// Update the store configuration (sync timeout, etc.).
    pub fn set_store_config(&mut self, config: crate::config::StoreConfig) {
        self.store_config = config;
    }

    /// Update the embedding re-embed policy.
    pub fn set_reembed_on_tag_change(&mut self, reembed: bool) {
        self.reembed_on_tag_change = reembed;
    }

    /// Update the resource limits and GC config for capacity warnings and auto-GC.
    pub fn set_resource_limits(&mut self, limits: crate::config::ResourceLimitsConfig, gc: crate::config::GcConfig) {
        self.resource_limits = limits;
        self.gc_config = gc;
    }

    fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Inject an integer `ref` field alongside the `id` field in a JSON object.
    ///
    /// The ref is assigned idempotently via UuidRefMap — the same UUID always
    /// gets the same integer ref within a session. Safe to call multiple times.
    fn inject_ref(&self, obj: &mut serde_json::Value) {
        if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
            let r = self.ref_map.assign_ref(id);
            obj.as_object_mut().map(|m| m.insert("ref".to_string(), json!(r)));
        }
    }

    /// Returns the tool router with `_meta.allowed_callers` injected into sandbox-safe tools.
    ///
    /// CEX-03: `search_memory` and `store_memory` are annotated as callable from
    /// `code_execution_20260120` sandboxes. Destructive tools (`delete_memory`,
    /// `bulk_delete_memories`) are intentionally excluded.
    fn tool_router_with_meta() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        let mut router = Self::tool_router();
        let sandbox_meta = {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "allowed_callers".to_string(),
                serde_json::json!(["direct", "code_execution_20260120"]),
            );
            Meta(obj)
        };
        for name in &["search_memory", "store_memory", "recall_memory", "annotate_memory"] {
            if let Some(route) = router.map.get_mut(*name) {
                route.attr.meta = Some(sandbox_meta.clone());
            }
        }
        router
    }
}

// Parameter structs

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StoreMemoryParams {
    /// Memory content
    pub content: String,
    /// Classification hint
    pub type_hint: Option<String>,
    /// Origin source
    pub source: Option<String>,
    /// Tags
    pub tags: Option<Vec<String>>,
    /// Actor identity
    pub actor: Option<String>,
    /// Actor type
    pub actor_type: Option<String>,
    /// Audience scope
    pub audience: Option<String>,
    /// Optional caller-provided idempotency key for at-most-once store semantics.
    /// Repeated calls with the same key return the original memory (first wins).
    /// When absent, content-hash dedup applies within the server's dedup window.
    pub idempotency_key: Option<String>,
    /// When true, block until embedding completes (or timeout). Default: false (async).
    /// Returns enriched response with embedding_status and embedding_dimension on completion.
    #[serde(default)]
    pub wait: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetMemoryParams {
    /// Memory ID
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateMemoryParams {
    /// Memory ID
    pub id: String,
    /// New content
    pub content: Option<String>,
    /// New classification hint
    pub type_hint: Option<String>,
    /// New origin source
    pub source: Option<String>,
    /// New tags (replaces existing)
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeleteMemoryParams {
    /// Memory ID
    pub id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BulkDeleteMemoriesParams {
    /// Filter by type_hint
    pub type_hint: Option<String>,
    /// Filter by source
    pub source: Option<String>,
    /// Created after (ISO-8601)
    pub created_after: Option<String>,
    /// Created before (ISO-8601)
    pub created_before: Option<String>,
    /// Updated after (ISO-8601)
    pub updated_after: Option<String>,
    /// Updated before (ISO-8601)
    pub updated_before: Option<String>,
    /// true to delete, false for count
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListMemoriesParams {
    /// Filter by type_hint
    pub type_hint: Option<String>,
    /// Filter by source
    pub source: Option<String>,
    /// Created after (ISO-8601)
    pub created_after: Option<String>,
    /// Created before (ISO-8601)
    pub created_before: Option<String>,
    /// Updated after (ISO-8601)
    pub updated_after: Option<String>,
    /// Updated before (ISO-8601)
    pub updated_before: Option<String>,
    /// Max results (1-100)
    pub limit: Option<u32>,
    /// Pagination cursor
    pub cursor: Option<String>,
    /// Filter by actor
    pub actor: Option<String>,
    /// Filter by audience
    pub audience: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReinforceMemoryParams {
    /// Memory ID
    pub id: String,
    /// Strength: "good" or "easy"
    #[serde(default = "default_rating")]
    pub rating: Option<String>,
}

fn default_rating() -> Option<String> {
    Some("good".to_string())
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FeedbackMemoryParams {
    /// Memory ID to provide feedback for
    pub id: String,
    /// Feedback signal: "useful" or "irrelevant"
    pub signal: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AnnotateMemoryParams {
    /// Memory ID to annotate
    pub id: String,
    /// Tags to append to existing tags (merged, deduplicated)
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Tags to replace ALL existing tags (overrides `tags` field if both given)
    #[serde(default)]
    pub replace_tags: Option<Vec<String>>,
    /// Salience value — absolute number (e.g., "0.9") or multiplier with "x" suffix (e.g., "1.5x").
    /// Absolute: sets stability directly. Multiplier: multiplies current stability.
    /// Examples: "0.9" (set to 0.9), "1.5x" (multiply by 1.5), "2.0x" (double)
    #[serde(default)]
    pub salience: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallMemoryParams {
    /// Query text to find relevant memories. Omit for query-less cold-start recall (returns top memories by salience).
    pub query: Option<String>,
    /// Session ID for dedup tracking. Auto-generated if omitted; return value includes session_id.
    pub session_id: Option<String>,
    /// Set to true to clear session recall history (e.g., after context compaction).
    pub reset: Option<bool>,
    /// Set to true for session-start context injection. Pins project-summary memory (if exists) and adds preamble/datetime.
    pub first: Option<bool>,
    /// Override max_memories config. Controls how many memories to return (not counting pinned summary).
    pub limit: Option<usize>,
    /// Optional boost tags for tag-affinity ranking. Memories sharing these tags get a soft ranking bonus.
    /// Prefix matching: "channel:" boosts all channel:* tags. Multiple tags combine additively.
    pub boost_tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SearchMemoryParams {
    /// Search query
    pub query: String,
    /// Max results (1-100)
    pub limit: Option<u32>,
    /// Created after (ISO-8601)
    pub created_after: Option<String>,
    /// Created before (ISO-8601)
    pub created_before: Option<String>,
    /// Filter by tags (all must match)
    pub tags: Option<Vec<String>>,
    /// Pagination cursor
    pub cursor: Option<String>,
    /// BM25 weight (0=disable, 1=default)
    pub bm25_weight: Option<f64>,
    /// Vector weight (0=disable, 1=default)
    pub vector_weight: Option<f64>,
    /// Symbolic weight (0=disable, 1=default)
    pub symbolic_weight: Option<f64>,
    /// Filter by audience
    pub audience: Option<String>,
    /// Field projection — return only these fields (e.g. ["id","content","tags"]).
    /// Omitting returns all fields (backwards compatible).
    pub fields: Option<Vec<String>>,
    /// Minimum salience score (0.0-1.0). Results below this threshold are excluded.
    /// Omitting applies config default_min_salience, or no filtering if that is also unset.
    pub min_salience: Option<f64>,
    /// Project scope — returns memories from this project plus global (null-project) memories.
    /// Omitting returns all memories regardless of project (no filtering).
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DiscoverMemoriesParams {
    /// Topic or concept to explore connections for
    pub query: String,
    /// Minimum cosine similarity (default 0.3). Lower = more surprising connections.
    pub min_similarity: Option<f64>,
    /// Maximum cosine similarity (default 0.7). Higher = more obviously related.
    pub max_similarity: Option<f64>,
    /// Maximum number of results (default 10)
    pub limit: Option<u32>,
    /// Project scope filter
    pub project: Option<String>,
}

// Helper: convert MemcpError to CallToolResult with isError: true
fn store_error_to_result(err: MemcpError) -> CallToolResult {
    match err {
        MemcpError::NotFound { id } => {
            CallToolResult::structured_error(json!({
                "isError": true,
                "error": format!("Memory not found: {}", id),
                "hint": "Use list_memories to find available memory IDs"
            }))
        }
        MemcpError::Validation { message, field } => {
            let mut obj = json!({
                "isError": true,
                "error": message,
            });
            if let Some(f) = field {
                obj["field"] = json!(f);
            }
            CallToolResult::structured_error(obj)
        }
        MemcpError::Storage(msg) => {
            CallToolResult::structured_error(json!({
                "isError": true,
                "error": format!("Storage error: {}", msg)
            }))
        }
        other => {
            CallToolResult::structured_error(json!({
                "isError": true,
                "error": other.to_string()
            }))
        }
    }
}

// Helper: parse optional ISO-8601 string to DateTime<Utc>
fn parse_datetime(s: &str, field: &str) -> Result<chrono::DateTime<chrono::Utc>, CallToolResult> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|_| {
            CallToolResult::structured_error(json!({
                "isError": true,
                "error": format!("Invalid datetime format for '{}': expected ISO-8601 (e.g. 2026-02-17T00:00:00Z)", field),
                "field": field
            }))
        })
}

// Helper: apply field projection to a search result JSON object.
//
// When `fields` is None or empty, returns the object unchanged (backwards compatible).
// When `fields` is Some(vec), returns only the keys present in the vec.
// Unknown field names are silently ignored (forward-compatible — callers won't break when
// new fields are added to the schema).
/// Apply field projection to a JSON object.
///
/// Supports one-level dot-notation (e.g., "metadata.source" extracts
/// `{ "metadata": { "source": ... } }`). Deeper paths (more than one dot)
/// are silently ignored. Non-object parents are silently skipped.
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
                        // Reject deeper paths (more than one dot): silently skip
                        if child_key.contains('.') {
                            continue;
                        }
                        if let Some(parent_val) = map.get(parent_key) {
                            if let serde_json::Value::Object(nested) = parent_val {
                                if let Some(child_val) = nested.get(child_key) {
                                    let entry = result.entry(parent_key.to_string())
                                        .or_insert_with(|| serde_json::json!({}));
                                    if let serde_json::Value::Object(ref mut m) = entry {
                                        m.insert(child_key.to_string(), child_val.clone());
                                    }
                                }
                            }
                        }
                    } else {
                        // Simple top-level field (existing behavior)
                        if let Some(val) = map.get(field.as_str()) {
                            result.insert(field.clone(), val.clone());
                        }
                    }
                }
                serde_json::Value::Object(result)
            } else {
                obj
            }
        }
    }
}

// Tool implementations
#[rmcp::tool_router]
impl MemoryService {
    #[tool(description = "Store a new memory. Returns {\"id\": \"uuid\", \"message\": \"Memory stored\"}.\n\
Params: content (required), type_hint (fact|preference|instruction|decision), \
tags (array), source (string), actor (string), actor_type (agent|human|system, default agent), \
audience (global|personal|team:X), idempotency_key (optional string), \
wait (bool, default false — when true, blocks until embedding completes; returns embedding_status and embedding_dimension).\n\
Dedup: identical content within the server dedup window returns the existing memory (no duplicate). \
Optional idempotency_key for caller-controlled at-most-once semantics — same key always returns original result.\n\
Callable from code_execution_20260120 sandboxes.")]
    async fn store_memory(
        &self,
        Parameters(params): Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "store_memory",
            type_hint = ?params.type_hint,
            source = ?params.source,
            "Tool called"
        );

        if params.content.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'content' is required and cannot be empty",
                "field": "content"
            })));
        }

        // Validate idempotency_key length
        if let Some(ref key) = params.idempotency_key {
            if key.len() > self.idempotency_config.max_key_length {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!(
                        "Field 'idempotency_key' exceeds maximum length of {} bytes",
                        self.idempotency_config.max_key_length
                    ),
                    "field": "idempotency_key"
                })));
            }
        }

        // Content filter check (before storage)
        if let Some(ref filter) = self.content_filter {
            match filter.check(&params.content).await {
                Ok(FilterVerdict::Allow) => {}
                Ok(FilterVerdict::Drop { reason }) => {
                    tracing::info!(reason = %reason, "store_memory: content filtered, not storing");
                    return Ok(CallToolResult::structured(json!({
                        "filtered": true,
                        "reason": reason,
                        "hint": "Content was not stored due to server content filtering rules"
                    })));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "store_memory: content filter error, proceeding with store");
                }
            }
        }

        // Resource cap: max_memories — hard reject at hard_cap_percent (default 110%)
        if let Some(max) = self.resource_caps.max_memories {
            if let Some(ref pg) = self.pg_store {
                match pg.count_live_memories().await {
                    Ok(count) => {
                        let ratio = count as f64 / max as f64;
                        let hard_cap = self.resource_limits.hard_cap_percent as f64 / 100.0;
                        if ratio >= hard_cap {
                            return Ok(CallToolResult::structured_error(json!({
                                "isError": true,
                                "error": format!("Resource cap exceeded: max_memories (limit: {}, current: {}, hard_cap: {}%)", max, count, self.resource_limits.hard_cap_percent),
                                "cap": "max_memories",
                                "limit": max,
                                "current": count,
                            })));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to check memory count for cap enforcement — proceeding");
                    }
                }
            }
        }

        let input = CreateMemory {
            content: params.content,
            type_hint: params.type_hint.unwrap_or_else(|| "fact".to_string()),
            source: params.source.unwrap_or_else(|| "default".to_string()),
            tags: params.tags,
            created_at: None,
            actor: params.actor,
            actor_type: params.actor_type.unwrap_or_else(|| "agent".to_string()),
            audience: params.audience.unwrap_or_else(|| "global".to_string()),
            idempotency_key: params.idempotency_key,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: None,
            session_id: None,
            agent_role: None,
        };

        // Determine if sync store is requested
        let sync_store = params.wait.unwrap_or(false);

        match self.store.store(input).await {
            Ok(memory) => {
                // Seed salience: explicit stores get stability=3.0 (stronger than auto-store's 2.5)
                if let Some(ref pg) = self.pg_store {
                    if let Err(e) = pg.upsert_salience(&memory.id, 3.0, 5.0, 0, None).await {
                        tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience for explicit store");
                    }
                }

                // Create oneshot channel for sync store (if requested)
                let (completion_tx, completion_rx) = if sync_store {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    (Some(tx), Some(rx))
                } else {
                    (None, None)
                };

                // Enqueue background embedding job
                if let Some(ref pipeline) = self.pipeline {
                    let text = crate::embedding::build_embedding_text(&memory.content, &memory.tags);
                    pipeline.enqueue(EmbeddingJob {
                        memory_id: memory.id.clone(),
                        text,
                        attempt: 0,
                        completion_tx,
                        tier: "fast".to_string(),
                    });
                }
                // Enqueue background extraction job (non-blocking, never sync)
                if let Some(ref extraction_pipeline) = self.extraction_pipeline {
                    extraction_pipeline.enqueue(ExtractionJob {
                        memory_id: memory.id.clone(),
                        content: memory.content.clone(),
                        attempt: 0,
                    });
                }

                // Build response
                let mut response_obj = json!({
                    "id": memory.id,
                    "content": memory.content,
                    "type_hint": memory.type_hint,
                    "source": memory.source,
                    "tags": memory.tags,
                    "created_at": memory.created_at.to_rfc3339(),
                    "updated_at": memory.updated_at.to_rfc3339(),
                    "access_count": memory.access_count,
                    "embedding_status": memory.embedding_status,
                    "actor": memory.actor,
                    "actor_type": memory.actor_type,
                    "audience": memory.audience,
                    "hint": "Use get_memory with this ID to retrieve, or update_memory to modify"
                });

                // Sync store: wait for embedding completion
                if let Some(rx) = completion_rx {
                    let timeout = Duration::from_secs(self.store_config.sync_timeout_secs);
                    match tokio::time::timeout(timeout, rx).await {
                        Ok(Ok(completion)) => {
                            response_obj["embedding_status"] = json!(completion.status);
                            if let Some(dim) = completion.dimension {
                                response_obj["embedding_dimension"] = json!(dim);
                            }
                        }
                        _ => {
                            // Timeout or channel error — embedding still pending
                            response_obj["embedding_status"] = json!("pending");
                        }
                    }
                }

                // Capacity warning: check memory count vs limits
                if let Some(max) = self.resource_caps.max_memories {
                    if let Some(ref pg) = self.pg_store {
                        if let Ok(count) = pg.count_live_memories().await {
                            let ratio = count as f64 / max as f64;
                            let warn_threshold = self.resource_limits.warn_percent as f64 / 100.0;
                            if ratio >= warn_threshold {
                                response_obj["warning"] = json!(format!(
                                    "Memory usage at {}%. Upgrade storage at engram.host/upgrade",
                                    (ratio * 100.0).round()
                                ));
                                // Auto-GC trigger (fire-and-forget with cooldown)
                                if self.resource_limits.auto_gc {
                                    let should_gc = {
                                        let mut last = self.last_auto_gc.lock().unwrap();
                                        let cooldown = Duration::from_secs(self.resource_limits.auto_gc_cooldown_mins * 60);
                                        match *last {
                                            Some(t) if t.elapsed() < cooldown => false,
                                            _ => { *last = Some(Instant::now()); true }
                                        }
                                    };
                                    if should_gc {
                                        let store = pg.clone();
                                        let gc_config = self.gc_config.clone();
                                        tokio::spawn(async move {
                                            tracing::info!("Auto-GC triggered (capacity near limit)");
                                            if let Err(e) = crate::gc::worker::run_gc(&store, &gc_config, false).await {
                                                tracing::warn!(error = %e, "Auto-GC failed");
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // Inject integer ref alongside UUID id
                self.inject_ref(&mut response_obj);

                Ok(CallToolResult::structured(response_obj))
            }
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Get a memory by ID.")]
    async fn get_memory(
        &self,
        Parameters(params): Parameters<GetMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "get_memory",
            id = %params.id,
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        match self.store.get(&id).await {
            Ok(memory) => {
                // Implicit salience bump on direct retrieval (fire-and-forget, not on search results)
                if let Some(ref pg_store) = self.pg_store {
                    let store = pg_store.clone();
                    let id_clone = id.clone();
                    tokio::spawn(async move {
                        if let Err(e) = store.touch_salience(&id_clone).await {
                            tracing::warn!("Failed to touch salience for {}: {}", id_clone, e);
                        }
                    });
                }
                let mut obj = json!({
                    "id": memory.id,
                    "content": memory.content,
                    "type_hint": memory.type_hint,
                    "source": memory.source,
                    "tags": memory.tags,
                    "created_at": memory.created_at.to_rfc3339(),
                    "updated_at": memory.updated_at.to_rfc3339(),
                    "last_accessed_at": memory.last_accessed_at.map(|dt| dt.to_rfc3339()),
                    "access_count": memory.access_count,
                    "embedding_status": memory.embedding_status,
                    "actor": memory.actor,
                    "actor_type": memory.actor_type,
                    "audience": memory.audience,
                    "hint": "Use update_memory to modify or delete_memory to remove"
                });
                self.inject_ref(&mut obj);
                Ok(CallToolResult::structured(obj))
            }
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Update a memory. At least one field required.")]
    async fn update_memory(
        &self,
        Parameters(params): Parameters<UpdateMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "update_memory",
            id = %params.id,
            has_content = params.content.is_some(),
            has_type_hint = params.type_hint.is_some(),
            has_source = params.source.is_some(),
            has_tags = params.tags.is_some(),
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        if params.content.is_none()
            && params.type_hint.is_none()
            && params.source.is_none()
            && params.tags.is_none()
        {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "At least one of 'content', 'type_hint', 'source', or 'tags' must be provided"
            })));
        }

        // Content filter check (only when content is changing)
        if let Some(ref new_content) = params.content {
            if let Some(ref filter) = self.content_filter {
                match filter.check(new_content).await {
                    Ok(FilterVerdict::Allow) => {}
                    Ok(FilterVerdict::Drop { reason }) => {
                        tracing::info!(reason = %reason, "update_memory: new content filtered, rejecting update");
                        return Ok(CallToolResult::structured(json!({
                            "filtered": true,
                            "reason": reason,
                            "hint": "Content update was rejected due to server content filtering rules"
                        })));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "update_memory: content filter error, proceeding with update");
                    }
                }
            }
        }

        // Track if content or tags changed — determines if re-embedding is needed
        let content_changed = params.content.is_some();
        let tags_changed = params.tags.is_some();

        let input = UpdateMemory {
            content: params.content,
            type_hint: params.type_hint,
            source: params.source,
            tags: params.tags,
            trust_level: None,
        };

        match self.store.update(&id, input).await {
            Ok(memory) => {
                // Re-embed when content changes. Tag-only changes skip re-embed by default
                // (configurable via embedding.reembed_on_tag_change in memcp.toml).
                let should_reembed = content_changed || (tags_changed && self.reembed_on_tag_change);
                if should_reembed {
                    if let Some(ref pipeline) = self.pipeline {
                        let text = crate::embedding::build_embedding_text(&memory.content, &memory.tags);
                        pipeline.enqueue(EmbeddingJob {
                            memory_id: memory.id.clone(),
                            text,
                            attempt: 0,
                            completion_tx: None,
                            tier: "fast".to_string(),
                        });
                    }
                }
                // Re-extract when content changes (extraction is content-only, not tags)
                if content_changed {
                    if let Some(ref extraction_pipeline) = self.extraction_pipeline {
                        // Reset extraction status to pending, then enqueue
                        if let Some(ref pg_store) = self.pg_store {
                            let store = pg_store.clone();
                            let mem_id = memory.id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = store.update_extraction_status(&mem_id, "pending").await {
                                    tracing::warn!("Failed to reset extraction status for {}: {}", mem_id, e);
                                }
                            });
                        }
                        extraction_pipeline.enqueue(ExtractionJob {
                            memory_id: memory.id.clone(),
                            content: memory.content.clone(),
                            attempt: 0,
                        });
                    }
                }
                let mut obj = json!({
                    "id": memory.id,
                    "content": memory.content,
                    "type_hint": memory.type_hint,
                    "source": memory.source,
                    "tags": memory.tags,
                    "created_at": memory.created_at.to_rfc3339(),
                    "updated_at": memory.updated_at.to_rfc3339(),
                    "access_count": memory.access_count,
                    "embedding_status": memory.embedding_status,
                    "actor": memory.actor,
                    "actor_type": memory.actor_type,
                    "audience": memory.audience,
                    "hint": "Use get_memory to re-read or delete_memory to remove"
                });
                self.inject_ref(&mut obj);
                Ok(CallToolResult::structured(obj))
            }
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Delete a memory by ID. Idempotent: returns success even if the memory does not exist (safe to retry).")]
    async fn delete_memory(
        &self,
        Parameters(params): Parameters<DeleteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "delete_memory",
            id = %params.id,
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        match self.store.delete(&id).await {
            Ok(()) => Ok(CallToolResult::structured(json!({
                "deleted": true,
                "id": id,
                "hint": "Memory permanently removed. Use store_memory to create new memories."
            }))),
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Bulk delete by filter. confirm=false returns count, confirm=true deletes.")]
    async fn bulk_delete_memories(
        &self,
        Parameters(params): Parameters<BulkDeleteMemoriesParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "bulk_delete_memories",
            confirm = params.confirm,
            type_hint = ?params.type_hint,
            source = ?params.source,
            "Tool called"
        );

        // Parse optional datetime strings
        let created_after = if let Some(ref s) = params.created_after {
            match parse_datetime(s, "created_after") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let created_before = if let Some(ref s) = params.created_before {
            match parse_datetime(s, "created_before") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let updated_after = if let Some(ref s) = params.updated_after {
            match parse_datetime(s, "updated_after") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let updated_before = if let Some(ref s) = params.updated_before {
            match parse_datetime(s, "updated_before") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let filter = ListFilter {
            type_hint: params.type_hint,
            source: params.source,
            created_after,
            created_before,
            updated_after,
            updated_before,
            ..ListFilter::default()
        };

        if !params.confirm {
            match self.store.count_matching(&filter).await {
                Ok(count) => Ok(CallToolResult::structured(json!({
                    "matched": count,
                    "deleted": false,
                    "hint": format!("Call bulk_delete_memories again with confirm: true to delete these {} memories", count)
                }))),
                Err(e) => Ok(store_error_to_result(e)),
            }
        } else {
            match self.store.delete_matching(&filter).await {
                Ok(count) => Ok(CallToolResult::structured(json!({
                    "deleted": count,
                    "confirmed": true,
                    "hint": "Bulk deletion complete. Use list_memories to verify."
                }))),
                Err(e) => Ok(store_error_to_result(e)),
            }
        }
    }

    #[tool(description = "List memories with filters and pagination.")]
    async fn list_memories(
        &self,
        Parameters(params): Parameters<ListMemoriesParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "list_memories",
            type_hint = ?params.type_hint,
            source = ?params.source,
            limit = ?params.limit,
            has_cursor = params.cursor.is_some(),
            "Tool called"
        );

        let limit = params.limit.unwrap_or(20).clamp(1, 100);

        // Parse optional datetime strings
        let created_after = if let Some(ref s) = params.created_after {
            match parse_datetime(s, "created_after") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let created_before = if let Some(ref s) = params.created_before {
            match parse_datetime(s, "created_before") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let updated_after = if let Some(ref s) = params.updated_after {
            match parse_datetime(s, "updated_after") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let updated_before = if let Some(ref s) = params.updated_before {
            match parse_datetime(s, "updated_before") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let filter = ListFilter {
            type_hint: params.type_hint,
            source: params.source,
            created_after,
            created_before,
            updated_after,
            updated_before,
            limit: limit as i64,
            cursor: params.cursor,
            actor: params.actor,
            audience: params.audience,
            project: None, // MCP list_memories doesn't expose project yet
            session_id: None,
            agent_role: None,
        };

        match self.store.list(filter).await {
            Ok(result) => {
                let memories: Vec<serde_json::Value> = result
                    .memories
                    .iter()
                    .map(|m| {
                        let mut obj = json!({
                            "id": m.id,
                            "content": m.content,
                            "type_hint": m.type_hint,
                            "source": m.source,
                            "tags": m.tags,
                            "created_at": m.created_at.to_rfc3339(),
                            "updated_at": m.updated_at.to_rfc3339(),
                            "access_count": m.access_count,
                            "embedding_status": m.embedding_status,
                            "actor": m.actor,
                            "actor_type": m.actor_type,
                            "audience": m.audience,
                        });
                        self.inject_ref(&mut obj);
                        obj
                    })
                    .collect();

                let count = memories.len();
                let has_more = result.next_cursor.is_some();

                Ok(CallToolResult::structured(json!({
                    "memories": memories,
                    "count": count,
                    "next_cursor": result.next_cursor,
                    "has_more": has_more,
                    "hint": "Use next_cursor value in cursor parameter to get next page"
                })))
            }
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Search memories by meaning. Returns salience-ranked results.\n\
Params: query (required), limit (1-100, default 20), fields (array of field names for projection — \
supports one-level dot-notation e.g. 'metadata.source'), \
min_salience (0.0-1.0, server-side quality filter), cursor (pagination token), \
tags (array, all must match), audience, created_after/created_before (ISO-8601), \
bm25_weight/vector_weight/symbolic_weight (0-1).\n\
Default output: {\"memories\": [{\"id\": \"uuid\", \"content\": \"text\", \"type_hint\": \"fact\", \
\"source\": \"default\", \"tags\": [\"t1\"], \"created_at\": \"ISO8601\", \"updated_at\": \"ISO8601\", \
\"access_count\": 0, \"relevance_score\": 0.85, \"composite_score\": 0.85, \"match_source\": \"hybrid\", \
\"rrf_score\": 0.031, \"actor\": null, \"actor_type\": \"agent\", \"audience\": \"global\"}], \
\"total_results\": 1, \"query\": \"...\", \"next_cursor\": \"...\", \"has_more\": false}.\n\
composite_score is a 0-1 blended relevance combining retrieval similarity and memory importance.\n\
With fields=[\"id\",\"content\"]: each result has only {\"id\": \"uuid\", \"content\": \"text\"}.\n\
Idempotent: identical queries always return consistent results (safe to retry).\n\
Callable from code_execution_20260120 sandboxes.")]
    async fn search_memory(
        &self,
        Parameters(params): Parameters<SearchMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "search_memory",
            query = %params.query,
            limit = ?params.limit,
            has_cursor = params.cursor.is_some(),
            "Tool called"
        );

        // 1. Validate query
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'query' is required and cannot be empty",
                "field": "query"
            })));
        }

        // 2. Validate limit (default 20 per CONTEXT.md), clamped to resource cap
        let user_limit = params.limit.unwrap_or(20).clamp(1, 100);
        let limit = std::cmp::min(
            user_limit as i64,
            self.resource_caps.max_search_results,
        ) as u32;

        // 2a. Validate min_salience and compute effective threshold.
        if let Some(ms) = params.min_salience {
            if !(0.0..=1.0).contains(&ms) {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "min_salience must be between 0.0 and 1.0",
                    "field": "min_salience"
                })));
            }
        }
        let search_config = &self.search_config;
        let effective_min = params.min_salience
            .or(search_config.default_min_salience)
            .unwrap_or(0.0);

        // 2b. Decode cursor if provided — get (last_salience_score, last_id) for keyset pagination.
        // cursor takes precedence; if both cursor and offset would have been present, cursor wins.
        let cursor_position: Option<(f64, String)> = if let Some(ref c) = params.cursor {
            match decode_search_keyset_cursor(c) {
                Ok(pos) => Some(pos),
                Err(e) => {
                    return Ok(CallToolResult::structured_error(json!({
                        "isError": true,
                        "error": format!("Invalid cursor: {}", e),
                        "field": "cursor"
                    })));
                }
            }
        } else {
            None
        };

        // 3. Get concrete PostgresMemoryStore reference (required for hybrid search)
        let pg_store = match &self.pg_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "Search requires PostgreSQL backend",
                    "hint": "Use list_memories to browse memories"
                })));
            }
        };

        // 4. Query Intelligence: decompose (if enabled) — replaces expand()
        let qi_start = Instant::now();
        let qi_budget = Duration::from_millis(self.qi_config.latency_budget_ms);

        // decomposed_meta: (is_multi_faceted, sub_queries_for_debug)
        let mut decomposed_meta: Option<(bool, Vec<String>)> = None;
        let (search_query, qi_time_range, sub_queries_for_search) =
            if let Some(ref provider) = self.qi_expansion_provider {
                let decompose_budget = qi_budget * 6 / 10; // 60% for decomposition
                match tokio::time::timeout(decompose_budget, provider.decompose(&params.query)).await {
                    Ok(Ok(dq)) => {
                        if dq.is_multi_faceted && !dq.sub_queries.is_empty() && self.qi_config.multi_query_enabled {
                            tracing::info!(
                                sub_query_count = dq.sub_queries.len(),
                                has_time_range = dq.time_range.is_some(),
                                "Query decomposed into sub-queries (multi-query path)"
                            );
                            decomposed_meta = Some((true, dq.sub_queries.clone()));
                            let time_range = dq.time_range;
                            // sub_queries_for_search is non-empty → multi-query path
                            (params.query.clone(), time_range, dq.sub_queries)
                        } else {
                            tracing::info!(
                                variants = dq.variants.len(),
                                has_time_range = dq.time_range.is_some(),
                                "Query decomposed as simple (single-query path)"
                            );
                            decomposed_meta = Some((false, vec![]));
                            let best_query = dq.variants.into_iter().next().unwrap_or_else(|| params.query.clone());
                            let time_range = dq.time_range;
                            (best_query, time_range, vec![])
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "Query decomposition failed, using original query");
                        (params.query.clone(), None, vec![])
                    }
                    Err(_) => {
                        tracing::warn!(elapsed_ms = ?qi_start.elapsed().as_millis(), "Query decomposition timed out, using original query");
                        (params.query.clone(), None, vec![])
                    }
                }
            } else {
                // No LLM — try deterministic temporal fallback
                let time_range = parse_temporal_hint(&params.query, Utc::now());
                (params.query.clone(), time_range, vec![])
            };

        // 5. Optionally embed the search_query (graceful degradation to BM25-only if no provider)
        let query_embedding: Option<pgvector::Vector> = if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&search_query).await {
                Ok(vec) => Some(pgvector::Vector::from(vec)),
                Err(e) => {
                    tracing::warn!("Failed to embed search query, falling back to BM25-only: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // 6. Parse optional datetime params
        let created_after = if let Some(ref s) = params.created_after {
            match parse_datetime(s, "created_after") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        let created_before = if let Some(ref s) = params.created_before {
            match parse_datetime(s, "created_before") {
                Ok(dt) => Some(dt),
                Err(result) => return Ok(result),
            }
        } else {
            None
        };

        // 7. Convert weight params to per-leg k values for RRF fusion.
        //    Formula: k = base_k / weight (lower k = more top-result influence).
        //    weight=0.0 → None (skip leg entirely).
        //    weight=None → default k (1.0 = no change to base_k).
        const BM25_BASE_K: f64 = 60.0;
        const VECTOR_BASE_K: f64 = 60.0;
        const SYMBOLIC_BASE_K: f64 = 40.0;

        let bm25_k = match params.bm25_weight {
            Some(w) if w == 0.0 => None,          // disabled
            Some(w) => Some(BM25_BASE_K / w),     // weight=2.0 → k=30.0 (stronger influence)
            None => Some(BM25_BASE_K),             // default
        };
        let vector_k = match params.vector_weight {
            Some(w) if w == 0.0 => None,
            Some(w) => Some(VECTOR_BASE_K / w),
            None => Some(VECTOR_BASE_K),
        };
        let symbolic_k = match params.symbolic_weight {
            Some(w) if w == 0.0 => None,
            Some(w) => Some(SYMBOLIC_BASE_K / w),
            None => Some(SYMBOLIC_BASE_K),
        };

        // Validate that at least one search path is enabled
        if bm25_k.is_none() && vector_k.is_none() && symbolic_k.is_none() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "At least one search path must be enabled (bm25_weight, vector_weight, or symbolic_weight must be non-zero)",
            })));
        }

        // 8. Call hybrid_search — BM25 + vector + symbolic with three-way RRF fusion.
        // Fetch a larger candidate pool when using cursor pagination (need candidates beyond cursor pos).
        // Salience re-ranking happens after, then cursor filtering is applied application-side.
        let fetch_limit = if cursor_position.is_some() { limit as i64 * 5 } else { limit as i64 };
        let tags_slice: Option<Vec<String>> = params.tags.clone();

        // 8a. Multi-query path: run hybrid_search for each sub-query, fuse results via rrf_fuse_multi.
        let raw_hits = if !sub_queries_for_search.is_empty() {
            // Embed all sub-queries
            let mut sub_query_result_ranks: Vec<Vec<(String, i64)>> = Vec::new();
            for sub_q in &sub_queries_for_search {
                let sub_embedding = if let Some(ref ep) = self.embedding_provider {
                    match ep.embed(sub_q).await {
                        Ok(vec) => Some(pgvector::Vector::from(vec)),
                        Err(e) => {
                            tracing::warn!(sub_query = %sub_q, error = %e, "Failed to embed sub-query, using BM25 only for this leg");
                            None
                        }
                    }
                } else {
                    None
                };
                let sub_hits = match pg_store.hybrid_search(
                    sub_q,
                    sub_embedding.as_ref(),
                    fetch_limit,
                    created_after,
                    created_before,
                    tags_slice.as_deref(),
                    bm25_k,
                    vector_k,
                    symbolic_k,
                    None,
                    params.audience.as_deref(),
                    params.project.as_deref(),
                ).await {
                    Ok(hits) => hits,
                    Err(e) => {
                        tracing::warn!(sub_query = %sub_q, error = %e, "Sub-query search failed, skipping leg");
                        continue;
                    }
                };
                // Convert to (id, rank) pairs for rrf_fuse_multi
                let ranks: Vec<(String, i64)> = sub_hits
                    .into_iter()
                    .enumerate()
                    .map(|(i, hit)| (hit.memory.id, (i + 1) as i64))
                    .collect();
                sub_query_result_ranks.push(ranks);
            }

            if sub_query_result_ranks.is_empty() {
                // All sub-query legs failed — fall back to original query
                tracing::warn!("All sub-query legs failed, falling back to original query");
                match pg_store.hybrid_search(
                    &params.query,
                    query_embedding.as_ref(),
                    fetch_limit,
                    created_after,
                    created_before,
                    tags_slice.as_deref(),
                    bm25_k,
                    vector_k,
                    symbolic_k,
                    None,
                    params.audience.as_deref(),
                    params.project.as_deref(),
                ).await {
                    Ok(hits) => hits,
                    Err(e) => return Ok(store_error_to_result(e)),
                }
            } else {
                // Fuse sub-query results via RRF
                const MULTI_QUERY_K: f64 = 60.0;
                let fused = rrf_fuse_multi(&sub_query_result_ranks, MULTI_QUERY_K);

                // Fetch full Memory objects for fused IDs (top fetch_limit)
                let fused_ids: Vec<String> = fused.into_iter()
                    .take(fetch_limit as usize)
                    .map(|(id, _)| id)
                    .collect();

                match pg_store.get_memories_by_ids(&fused_ids).await {
                    Ok(memory_map) => {
                        // Preserve fused rank order: iterate fused_ids in order, look up memory
                        fused_ids.iter().enumerate().filter_map(|(i, id)| {
                            memory_map.get(id).map(|mem| {
                                let rank = i + 1;
                                let rrf_score = 1.0 / (MULTI_QUERY_K + rank as f64);
                                crate::search::HybridRawHit {
                                    memory: mem.clone(),
                                    rrf_score,
                                    match_source: "multi_query".to_string(),
                                }
                            })
                        }).collect()
                    }
                    Err(e) => return Ok(store_error_to_result(e)),
                }
            }
        } else {
            // 8b. Single-query path (default)
            match pg_store.hybrid_search(
                &search_query,
                query_embedding.as_ref(),
                fetch_limit,
                created_after,
                created_before,
                tags_slice.as_deref(),
                bm25_k,
                vector_k,
                symbolic_k,
                None, // source filter (MCP uses separate params)
                params.audience.as_deref(),
                params.project.as_deref(),
            ).await {
                Ok(hits) => hits,
                Err(e) => return Ok(store_error_to_result(e)),
            }
        };

        // 9. Fetch salience data for all result IDs
        let ids: Vec<String> = raw_hits.iter().map(|h| h.memory.id.clone()).collect();
        let salience_data = match pg_store.get_salience_data(&ids).await {
            Ok(data) => data,
            Err(e) => return Ok(store_error_to_result(e)),
        };

        // 10. Build ScoredHit vec for salience re-ranking
        let mut scored_hits: Vec<ScoredHit> = raw_hits
            .into_iter()
            .map(|hit| ScoredHit {
                memory: hit.memory,
                rrf_score: hit.rrf_score,
                salience_score: 0.0, // populated by rank()
                match_source: hit.match_source,
                breakdown: None,     // populated by rank() when debug_scoring=true
                composite_score: 0.0, // populated after salience scoring
            })
            .collect();

        // 11. Build SalienceInput for each hit (parallel order to scored_hits)
        let salience_inputs: Vec<SalienceInput> = scored_hits
            .iter()
            .map(|hit| {
                let row = salience_data
                    .get(&hit.memory.id)
                    .cloned()
                    .unwrap_or_default();
                let days_since_reinforced = row.last_reinforced_at
                    .map(|dt| {
                        let duration = Utc::now().signed_duration_since(dt);
                        (duration.num_seconds() as f64 / 86_400.0).max(0.0)
                    })
                    .unwrap_or(365.0); // 1 year default for never-reinforced memories
                SalienceInput {
                    stability: row.stability,
                    days_since_reinforced,
                }
            })
            .collect();

        // 12. Apply salience re-ranking
        let scorer = SalienceScorer::new(&self.salience_config);
        scorer.rank(&mut scored_hits, &salience_inputs);

        // 12.5 Apply temporal soft boost if time range extracted
        // Uses event_time (content-referenced time) when present, falling back to created_at.
        // This means a memory stored today about "in 2019" correctly gets boosted when
        // searching for "2019 memories" — the event time takes precedence over store time.
        if let Some(ref time_range) = qi_time_range {
            for hit in &mut scored_hits {
                let t = hit.memory.event_time.unwrap_or(hit.memory.created_at);
                let in_range = match (time_range.after, time_range.before) {
                    (Some(after), Some(before)) => t >= after && t <= before,
                    (Some(after), None) => t >= after,
                    (None, Some(before)) => t <= before,
                    (None, None) => false,
                };
                if in_range {
                    hit.salience_score *= 2.0; // 2x boost for in-range memories (soft boost, not filter)
                }
            }
            // Re-sort by boosted salience score
            scored_hits.sort_by(|a, b| b.salience_score.partial_cmp(&a.salience_score).unwrap_or(std::cmp::Ordering::Equal));
        }

        // 12.75 LLM re-ranking (if enabled and budget remaining)
        if let Some(ref provider) = self.qi_reranking_provider {
            let remaining = qi_budget.saturating_sub(qi_start.elapsed());
            if remaining > Duration::from_millis(100) { // Only attempt if >100ms remains
                // Take top 10 for re-ranking (locked decision)
                let top_n = scored_hits.len().min(10);
                let candidates: Vec<RankedCandidate> = scored_hits[..top_n]
                    .iter()
                    .enumerate()
                    .map(|(i, hit)| {
                        let content = if hit.memory.content.len() > self.qi_config.rerank_content_chars {
                            hit.memory.content[..self.qi_config.rerank_content_chars].to_string()
                        } else {
                            hit.memory.content.clone()
                        };
                        RankedCandidate {
                            id: hit.memory.id.clone(),
                            content,
                            current_rank: i + 1,
                        }
                    })
                    .collect();

                match tokio::time::timeout(remaining, provider.rerank(&params.query, &candidates)).await {
                    Ok(Ok(ranked)) => {
                        tracing::info!(ranked_count = ranked.len(), "LLM re-ranking applied");
                        // Blend: 0.7 * llm_rank_score + 0.3 * salience_score (normalized)
                        // llm_rank_score = 1.0 / (1.0 + llm_rank as f64)
                        let max_salience = scored_hits.iter().map(|h| h.salience_score).fold(f64::MIN, f64::max);
                        let min_salience = scored_hits.iter().map(|h| h.salience_score).fold(f64::MAX, f64::min);
                        let salience_range = (max_salience - min_salience).max(1e-6);

                        for hit in scored_hits[..top_n].iter_mut() {
                            if let Some(r) = ranked.iter().find(|r| r.id == hit.memory.id) {
                                let llm_score = 1.0 / (1.0 + r.llm_rank as f64);
                                let norm_salience = (hit.salience_score - min_salience) / salience_range;
                                hit.salience_score = 0.7 * llm_score + 0.3 * norm_salience;
                            }
                        }
                        // Re-sort top_n portion only
                        scored_hits[..top_n].sort_by(|a, b| b.salience_score.partial_cmp(&a.salience_score).unwrap_or(std::cmp::Ordering::Equal));
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "LLM re-ranking failed, keeping salience order");
                    }
                    Err(_) => {
                        tracing::warn!(elapsed_ms = ?qi_start.elapsed().as_millis(), "LLM re-ranking timed out, keeping salience order");
                    }
                }
            } else {
                tracing::debug!(remaining_ms = ?remaining.as_millis(), "Skipping re-ranking — insufficient budget remaining");
            }
        }

        // 12.5. Apply salience threshold filtering AFTER re-ranking, BEFORE cursor/take.
        // Count below-threshold results first (needed for hint mode).
        let below_threshold = scored_hits.iter().filter(|h| h.salience_score < effective_min).count();
        let mut scored_hits: Vec<ScoredHit> = if effective_min > 0.0 {
            scored_hits.into_iter().filter(|h| h.salience_score >= effective_min).collect()
        } else {
            scored_hits
        };

        // 12.55. Compute composite relevance score (0-1) blending RRF and salience.
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
                // 50% RRF (retrieval relevance) + 50% salience (memory importance)
                hit.composite_score = 0.5 * norm_rrf + 0.5 * norm_sal;
            }
        }

        // 12.6. Deduplicate parent/chunk collisions — prefer chunks over parents.
        dedup_parent_chunks(&mut scored_hits);

        // 13. Apply cursor-based filtering: skip items at or before the cursor position.
        // Cursor encodes (salience_score, id) of the LAST item on the previous page.
        let scored_hits: Vec<ScoredHit> = if let Some((last_score, ref last_id)) = cursor_position {
            scored_hits.into_iter().filter(|hit| {
                let score = hit.salience_score;
                if (score - last_score).abs() < f64::EPSILON {
                    hit.memory.id.as_str() > last_id.as_str()
                } else {
                    score < last_score
                }
            }).collect()
        } else {
            scored_hits
        };

        // Trim to limit and detect if more remain.
        let has_more = scored_hits.len() as u32 > limit;
        let take = if has_more { limit as usize } else { scored_hits.len() };
        let scored_hits: Vec<ScoredHit> = scored_hits.into_iter().take(take).collect();

        // Build next_cursor from the last item's (salience_score, id).
        let next_cursor: Option<String> = if has_more {
            scored_hits.last().map(|hit| encode_search_keyset_cursor(hit.salience_score, &hit.memory.id))
        } else {
            None
        };

        // 14. Format results
        let count = scored_hits.len();
        let results: Vec<serde_json::Value> = scored_hits.iter().map(|hit| {
            let mut obj = json!({
                "id": hit.memory.id,
                "content": hit.memory.content,
                "type_hint": hit.memory.type_hint,
                "source": hit.memory.source,
                "tags": hit.memory.tags,
                "created_at": hit.memory.created_at.to_rfc3339(),
                "updated_at": hit.memory.updated_at.to_rfc3339(),
                "access_count": hit.memory.access_count,
                "relevance_score": (hit.salience_score * 1000.0).round() / 1000.0,
                "composite_score": (hit.composite_score * 1000.0).round() / 1000.0,
                "match_source": hit.match_source,
                "rrf_score": (hit.rrf_score * 10000.0).round() / 10000.0,
                "actor": hit.memory.actor,
                "actor_type": hit.memory.actor_type,
                "audience": hit.memory.audience,
            });
            // Add score breakdown when debug_scoring is enabled
            if let Some(ref bd) = hit.breakdown {
                obj["score_breakdown"] = json!({
                    "recency": (bd.recency * 1000.0).round() / 1000.0,
                    "access": (bd.access * 1000.0).round() / 1000.0,
                    "semantic": (bd.semantic * 1000.0).round() / 1000.0,
                    "reinforcement": (bd.reinforcement * 1000.0).round() / 1000.0,
                });
            }
            // Inject integer ref alongside UUID id (always present, even when field projection is used)
            self.inject_ref(&mut obj);
            // Apply field projection (no-op when fields is None or empty).
            apply_field_projection(obj, &params.fields)
        }).collect();

        // 15. Build final response JSON
        let mut response = json!({
            "memories": results,
            "total_results": count,
            "query": params.query,
            "next_cursor": next_cursor,
            "has_more": has_more,
        });

        // 15a. Add decomposition debug metadata when available
        if let Some((is_multi, sub_queries)) = decomposed_meta {
            response["decomposed"] = json!(is_multi);
            if is_multi && !sub_queries.is_empty() {
                response["sub_queries"] = json!(sub_queries);
            }
        }

        if count == 0 {
            response["hint"] = json!("No memories matched your query. Try broader search terms or use list_memories to browse all memories.");
            // Add salience hint when hint mode is enabled and results were filtered by threshold.
            if search_config.salience_hint_mode && below_threshold > 0 {
                response["salience_hint"] = json!(format!(
                    "{} results found below threshold {}",
                    below_threshold, effective_min
                ));
            }
        }

        Ok(CallToolResult::structured(response))
    }

    #[tool(description = "Reinforce a memory to boost future search salience.")]
    async fn reinforce_memory(
        &self,
        Parameters(params): Parameters<ReinforceMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "reinforce_memory",
            id = %params.id,
            rating = ?params.rating,
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        // Verify memory exists
        match self.store.get(&id).await {
            Err(MemcpError::NotFound { .. }) => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!("Memory not found: {}", id),
                    "hint": "Use list_memories to find available memory IDs"
                })));
            }
            Err(e) => return Ok(store_error_to_result(e)),
            Ok(_) => {}
        }

        // Validate and normalize rating
        let rating = params.rating.as_deref().unwrap_or("good");
        let rating = if rating == "easy" { "easy" } else { "good" };

        // Get concrete pg_store reference
        let pg_store = match &self.pg_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "Reinforcement requires PostgreSQL backend"
                })));
            }
        };

        match pg_store.reinforce_salience(&id, rating).await {
            Ok(row) => {
                let mut obj = json!({
                    "id": id,
                    "stability": row.stability,
                    "reinforcement_count": row.reinforcement_count,
                    "message": format!(
                        "Memory reinforced. Stability: {:.1} days, reinforcements: {}",
                        row.stability, row.reinforcement_count
                    )
                });
                self.inject_ref(&mut obj);
                Ok(CallToolResult::structured(obj))
            },
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Provide relevance feedback for a memory (useful or irrelevant). Adjusts salience scoring.")]
    async fn feedback_memory(
        &self,
        Parameters(params): Parameters<FeedbackMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "feedback_memory",
            id = %params.id,
            signal = %params.signal,
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        let pg_store = match &self.pg_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "Feedback requires PostgreSQL backend"
                })));
            }
        };

        match pg_store.apply_feedback(&id, &params.signal).await {
            Ok(()) => Ok(CallToolResult::structured(json!({ "ok": true }))),
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Recall relevant memories for automatic context injection. \
Query-based: provide 'query' to find semantically similar memories. \
Query-less: omit 'query' for cold-start recall ranked by salience (stability + recency). \
Set 'first' to true for session-start mode: pins project-summary memory and adds datetime/preamble. \
Returns {\"session_id\": \"...\", \"count\": N, \"memories\": [...], \"summary\": {...} | null}. \
Session-scoped dedup prevents re-injection within a conversation. \
Callable from code_execution_20260120 sandboxes.")]
    async fn recall_memory(
        &self,
        Parameters(params): Parameters<RecallMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "recall_memory",
            session_id = ?params.session_id,
            reset = params.reset.unwrap_or(false),
            first = params.first.unwrap_or(false),
            "Tool called"
        );

        // Get pg_store (required for recall session methods).
        let pg_store = match &self.pg_store {
            Some(s) => Arc::clone(s),
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "recall_memory requires PostgreSQL store"
                })));
            }
        };

        let engine = crate::recall::RecallEngine::new(
            pg_store.clone(),
            self.recall_config.clone(),
            self.extraction_enabled,
        );

        let first = params.first.unwrap_or(false);
        let reset = params.reset.unwrap_or(false);
        let boost_tags = params.boost_tags.unwrap_or_default();

        // Branch on query presence: Some(non-empty) → query-based, None or empty → queryless.
        let has_query = params.query.as_ref().map_or(false, |q| !q.trim().is_empty());

        let mut result = if has_query {
            // Query-based path — needs embedding provider.
            let embedding_provider = match &self.embedding_provider {
                Some(p) => p,
                None => {
                    return Ok(CallToolResult::structured_error(json!({
                        "isError": true,
                        "error": "recall_memory with a query requires an embedding provider"
                    })));
                }
            };

            let query = params.query.as_ref().unwrap();
            let query_embedding = match embedding_provider.embed(query).await {
                Ok(emb) => emb,
                Err(e) => {
                    return Ok(CallToolResult::structured_error(json!({
                        "isError": true,
                        "error": format!("Embedding failed: {}", e)
                    })));
                }
            };

            engine.recall(&query_embedding, params.session_id, reset, None, &boost_tags).await
        } else {
            // Query-less path — no embedding needed.
            engine.recall_queryless(params.session_id, reset, None, first, params.limit, &boost_tags).await
        };

        // For query-based path with first=true, fetch project summary separately.
        // (recall_queryless already handles this internally.)
        if has_query && first {
            if let Ok(ref mut r) = result {
                match pg_store.fetch_project_summary(None).await {
                    Ok(Some((id, content))) => {
                        r.summary = Some(crate::recall::RecalledMemory {
                            memory_id: id,
                            content,
                            relevance: 1.0,
                            boost_applied: false,
                            boost_score: 0.0,
                        });
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!(error = %e, "fetch_project_summary failed"),
                }
            }
        }

        match result {
            Ok(r) => {
                // Build memories array with ref injected alongside memory_id
                let memories: Vec<serde_json::Value> = r.memories.iter().map(|m| {
                    let r_num = self.ref_map.assign_ref(&m.memory_id);
                    let mut obj = json!({
                        "memory_id": m.memory_id,
                        "ref": r_num,
                        "content": m.content,
                        "relevance": m.relevance,
                    });
                    if m.boost_applied {
                        obj["boost_applied"] = json!(m.boost_applied);
                    }
                    if m.boost_score != 0.0 {
                        obj["boost_score"] = json!(m.boost_score);
                    }
                    obj
                }).collect();

                let mut response = json!({
                    "session_id": r.session_id,
                    "count": r.count,
                    "memories": memories,
                });
                // Include summary if present, with ref.
                if let Some(ref summary) = r.summary {
                    let summary_ref = self.ref_map.assign_ref(&summary.memory_id);
                    if let serde_json::Value::Object(ref mut map) = response {
                        map.insert("summary".to_string(), json!({
                            "memory_id": summary.memory_id,
                            "ref": summary_ref,
                            "content": summary.content,
                        }));
                    }
                }
                Ok(CallToolResult::structured(response))
            }
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Annotate an existing memory — add/replace tags and adjust salience. \
Tags: `tags` appends to existing (merged, deduplicated), `replace_tags` replaces all. \
Salience: \"0.9\" sets absolute stability, \"1.5x\" multiplies current. \
Returns diff showing changes. \
Example: {\"id\": \"abc\", \"tags\": [\"decision\"], \"salience\": \"1.5x\"}")]
    async fn annotate_memory(
        &self,
        Parameters(params): Parameters<AnnotateMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "annotate_memory",
            id = %params.id,
            "Tool called"
        );

        if params.id.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'id' is required and cannot be empty",
                "field": "id"
            })));
        }

        // Resolve integer ref or UUID passthrough
        let id = self.ref_map.resolve(&params.id)
            .unwrap_or_else(|| params.id.clone());

        let pg_store = match &self.pg_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "annotate_memory requires PostgreSQL backend"
                })));
            }
        };

        match crate::cli::annotate_logic(
            pg_store,
            &id,
            params.tags,
            params.replace_tags,
            params.salience,
        )
        .await
        {
            Ok(result) => {
                let mut changes = serde_json::Map::new();
                changes.insert("tags_added".to_string(), json!(result.tags_added));
                changes.insert("tags_removed".to_string(), json!(result.tags_removed));
                if let (Some(before), Some(after)) = (result.salience_before, result.salience_after) {
                    changes.insert("salience_before".to_string(), json!(before));
                    changes.insert("salience_after".to_string(), json!(after));
                }
                let mut obj = json!({
                    "id": result.id,
                    "changes": changes,
                });
                self.inject_ref(&mut obj);
                Ok(CallToolResult::structured(obj))
            }
            Err(e) => Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": e.to_string()
            }))),
        }
    }

    #[tool(description = "Discover unexpected connections between memories. \
Finds memories in the cosine similarity sweet spot (0.3-0.7) — related enough \
to be meaningful but different enough to be surprising. Use for creative \
exploration and lateral thinking, not for finding specific information (use search_memory for that).\n\
Returns results with similarity scores and optional LLM-generated connection explanations.")]
    async fn discover_memories(
        &self,
        Parameters(params): Parameters<DiscoverMemoriesParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "discover_memories",
            query = %params.query,
            min_similarity = ?params.min_similarity,
            max_similarity = ?params.max_similarity,
            limit = ?params.limit,
            "Tool called"
        );

        // Validate query
        if params.query.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'query' is required and cannot be empty",
                "field": "query"
            })));
        }

        let min_sim = params.min_similarity.unwrap_or(0.3).clamp(0.0, 1.0);
        let max_sim = params.max_similarity.unwrap_or(0.7).clamp(0.0, 1.0);
        let limit = params.limit.unwrap_or(10).clamp(1, 50);

        if min_sim >= max_sim {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "min_similarity must be less than max_similarity",
                "fields": ["min_similarity", "max_similarity"]
            })));
        }

        // Require embedding provider — discovery is vector-only
        let embedding_provider = match &self.embedding_provider {
            Some(p) => p,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "discover_memories requires an embedding provider"
                })));
            }
        };

        // Require pg_store
        let pg_store = match &self.pg_store {
            Some(s) => Arc::clone(s),
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "discover_memories requires PostgreSQL backend"
                })));
            }
        };

        // Embed query
        let embedding = match embedding_provider.embed(&params.query).await {
            Ok(emb) => pgvector::Vector::from(emb),
            Err(e) => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!("Embedding failed: {}", e)
                })));
            }
        };

        // Run discovery
        let results = match pg_store.discover_associations(
            &embedding,
            min_sim,
            max_sim,
            limit,
            params.project.as_deref(),
        ).await {
            Ok(r) => r,
            Err(e) => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!("Discovery failed: {}", e)
                })));
            }
        };

        // Optional LLM explanations (fail-open — no explanations if unavailable)
        let explanations = if let Some(ref provider) = self.qi_expansion_provider {
            let slices: Vec<(&str, f64)> = results.iter()
                .map(|(m, sim)| (m.content.as_str(), *sim))
                .collect();
            provider.explain_connections(&params.query, &slices).await.unwrap_or_default()
        } else {
            vec![]
        };

        // Build response with UUID ref mapping
        let discoveries: Vec<serde_json::Value> = results.iter().enumerate().map(|(i, (memory, sim))| {
            let mut obj = json!({
                "id": memory.id,
                "content": memory.content,
                "type_hint": memory.type_hint,
                "tags": memory.tags,
                "similarity": format!("{:.3}", sim),
                "created_at": memory.created_at.to_rfc3339(),
            });
            self.inject_ref(&mut obj);
            if let Some(explanation) = explanations.get(i) {
                if let Some(o) = obj.as_object_mut() {
                    o.insert("connection".to_string(), json!(explanation));
                }
            }
            obj
        }).collect();

        Ok(CallToolResult::structured(json!({
            "discoveries": discoveries,
            "query": params.query,
            "similarity_range": [min_sim, max_sim],
            "count": discoveries.len(),
        })))
    }

    #[tool(description = "Health check.")]
    async fn health_check(
        &self,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(tool = "health_check", "Tool called");

        let response = json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_seconds": self.uptime_seconds(),
        });

        Ok(CallToolResult::structured(response))
    }
}

// Helper: format a slice of memories into human-readable text for resource consumption
fn format_memories_text(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    memories
        .iter()
        .map(|m| {
            format!(
                "---\n[{}] {}\nCreated: {} | Source: {} | Accessed: {} times\n---",
                m.type_hint,
                m.content,
                m.created_at.to_rfc3339(),
                m.source,
                m.access_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ServerHandler implementation
#[rmcp::tool_handler(router = Self::tool_router_with_meta())]
impl ServerHandler for MemoryService {
    fn get_info(&self) -> rmcp::model::InitializeResult {
        rmcp::model::InitializeResult {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: "memcp".to_string(),
                title: None,
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some("High-performance MCP memory server with persistent PostgreSQL storage with semantic search".to_string()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Memory server for AI agents. Tools: store_memory, get_memory, search_memory, update_memory, delete_memory, bulk_delete_memories, list_memories, reinforce_memory, recall_memory, discover_memories, health_check.\n\nSearch uses keyword + semantic matching ranked by salience (recency, access frequency, relevance, reinforcement). Use list_memories to browse by filters. Reinforcement uses spaced repetition — faded memories get stronger boosts.\n\ndiscover_memories finds creative connections in the 0.3-0.7 similarity sweet spot — use for lateral thinking and inspiration, not exact retrieval.\n\nDefaults: type_hint=\"fact\", source=\"default\", actor_type=\"agent\", audience=\"global\". Weights: 0=disable leg, 1=default, >1=emphasize.".to_string()
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            meta: None,
            resources: vec![
                RawResource {
                    uri: "memory://session-primer".to_string(),
                    name: "session-primer".to_string(),
                    title: Some("Session Memory Primer".to_string()),
                    description: Some("Recent memories for session context".to_string()),
                    mime_type: Some("text/plain".to_string()),
                    size: None,
                    icons: None,
                    meta: None,
                }
                .no_annotation(),
                RawResource {
                    uri: "memory://user-profile".to_string(),
                    name: "user-profile".to_string(),
                    title: Some("User Profile".to_string()),
                    description: Some("User preferences and persistent facts".to_string()),
                    mime_type: Some("text/plain".to_string()),
                    size: None,
                    icons: None,
                    meta: None,
                }
                .no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match request.uri.as_str() {
            "memory://session-primer" => {
                let filter = ListFilter {
                    limit: 20,
                    ..Default::default()
                };
                let result = self
                    .store
                    .list(filter)
                    .await
                    .map_err(|e| McpError::resource_not_found(e.to_string(), None))?;

                let text = if result.memories.is_empty() {
                    "No memories stored yet. Use store_memory to add your first memory.".to_string()
                } else {
                    format_memories_text(&result.memories)
                };

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(text, request.uri)],
                })
            }
            "memory://user-profile" => {
                let filter = ListFilter {
                    type_hint: Some("preference".to_string()),
                    limit: 50,
                    ..Default::default()
                };
                let result = self
                    .store
                    .list(filter)
                    .await
                    .map_err(|e| McpError::resource_not_found(e.to_string(), None))?;

                let text = if result.memories.is_empty() {
                    "No user preferences stored yet. Use store_memory with type_hint: 'preference' to add preferences.".to_string()
                } else {
                    format_memories_text(&result.memories)
                };

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(text, request.uri)],
                })
            }
            uri => Err(McpError::resource_not_found(
                format!("Resource not found: {}", uri),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod uuid_ref_tests {
    use super::*;

    #[test]
    fn test_uuid_ref_idempotent() {
        let map = UuidRefMap::new();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let r1 = map.assign_ref(uuid);
        let r2 = map.assign_ref(uuid);
        assert_eq!(r1, r2, "Same UUID must always get the same ref");
    }

    #[test]
    fn test_uuid_ref_resolve_integer() {
        let map = UuidRefMap::new();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let r = map.assign_ref(uuid);
        let resolved = map.resolve(&r.to_string());
        assert_eq!(resolved, Some(uuid.to_string()), "Integer ref must resolve back to UUID");
    }

    #[test]
    fn test_uuid_ref_passthrough() {
        let map = UuidRefMap::new();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        // UUID passthrough — no assignment needed
        let resolved = map.resolve(uuid);
        assert_eq!(resolved, Some(uuid.to_string()), "UUID string must pass through as-is");
    }

    #[test]
    fn test_uuid_ref_resolve_unknown_integer() {
        let map = UuidRefMap::new();
        // Ref 999 was never assigned
        let resolved = map.resolve("999");
        assert_eq!(resolved, None, "Unknown integer ref must return None");
    }

    #[test]
    fn test_uuid_ref_starts_at_one() {
        let map = UuidRefMap::new();
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let r = map.assign_ref(uuid);
        assert_eq!(r, 1, "First assigned ref must be 1, not 0");
    }

    #[test]
    fn test_inject_ref_adds_ref_to_json() {
        let map = UuidRefMap::new();
        let uuid = "abc-123-def-456";
        let mut obj = json!({"id": uuid, "content": "test memory"});
        // Manually simulate inject_ref logic
        if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
            let r = map.assign_ref(id);
            obj.as_object_mut().unwrap().insert("ref".to_string(), json!(r));
        }
        assert!(obj.get("ref").is_some(), "inject_ref must add 'ref' field");
        assert_eq!(obj["ref"], json!(1u32), "First ref must be 1");
        assert_eq!(obj["id"], json!(uuid), "id field must be preserved");
    }

    #[test]
    fn test_inject_ref_array() {
        let map = UuidRefMap::new();
        let uuids = ["uuid-aaa", "uuid-bbb", "uuid-ccc"];
        let mut items: Vec<serde_json::Value> = uuids.iter().map(|u| json!({"id": u})).collect();
        // Simulate inject_ref on each element
        for item in &mut items {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                let r = map.assign_ref(id);
                item.as_object_mut().unwrap().insert("ref".to_string(), json!(r));
            }
        }
        // Each item should have a unique, sequential ref
        assert_eq!(items[0]["ref"], json!(1u32));
        assert_eq!(items[1]["ref"], json!(2u32));
        assert_eq!(items[2]["ref"], json!(3u32));
    }
}
