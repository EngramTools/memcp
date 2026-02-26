//! MCP server — rmcp-based tool handler for stdio transport.
//!
//! MemoryService implements ServerHandler with tools: store_memory, search_memory,
//! update_memory, delete_memory, list_memories, recall_memory, feedback_memory, etc.
//! Wires together storage/, intelligence/, and pipeline/ layers.

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

    fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
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
        for name in &["search_memory", "store_memory", "recall_memory"] {
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallMemoryParams {
    /// Query text to find relevant memories for context injection
    pub query: String,
    /// Session ID for dedup tracking. Auto-generated if omitted; return value includes session_id.
    pub session_id: Option<String>,
    /// Set to true to clear session recall history (e.g., after context compaction).
    pub reset: Option<bool>,
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
fn apply_field_projection(obj: serde_json::Value, fields: &Option<Vec<String>>) -> serde_json::Value {
    match fields {
        None => obj,
        Some(requested) if requested.is_empty() => obj,
        Some(requested) => {
            if let serde_json::Value::Object(map) = obj {
                let filtered: serde_json::Map<String, serde_json::Value> = map
                    .into_iter()
                    .filter(|(k, _)| requested.iter().any(|r| r == k))
                    .collect();
                serde_json::Value::Object(filtered)
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
audience (global|personal|team:X), idempotency_key (optional string).\n\
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

        // Resource cap: max_memories — fail-open on count query errors (Pitfall 4)
        if let Some(max) = self.resource_caps.max_memories {
            if let Some(ref pg) = self.pg_store {
                match pg.count_live_memories().await {
                    Ok(count) if count as u64 >= max => {
                        return Ok(CallToolResult::structured_error(json!({
                            "isError": true,
                            "error": format!("Resource cap exceeded: max_memories (limit: {}, current: {})", max, count),
                            "cap": "max_memories",
                            "limit": max,
                            "current": count,
                        })));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to check memory count for cap enforcement — proceeding");
                    }
                    _ => {}
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
        };

        match self.store.store(input).await {
            Ok(memory) => {
                // Seed salience: explicit stores get stability=3.0 (stronger than auto-store's 2.5)
                if let Some(ref pg) = self.pg_store {
                    if let Err(e) = pg.upsert_salience(&memory.id, 3.0, 5.0, 0, None).await {
                        tracing::warn!(error = %e, memory_id = %memory.id, "Failed to seed salience for explicit store");
                    }
                }
                // Enqueue background embedding job (non-blocking)
                if let Some(ref pipeline) = self.pipeline {
                    let text = crate::embedding::build_embedding_text(&memory.content, &memory.tags);
                    pipeline.enqueue(EmbeddingJob {
                        memory_id: memory.id.clone(),
                        text,
                        attempt: 0,
                    });
                }
                // Enqueue background extraction job (non-blocking)
                if let Some(ref extraction_pipeline) = self.extraction_pipeline {
                    extraction_pipeline.enqueue(ExtractionJob {
                        memory_id: memory.id.clone(),
                        content: memory.content.clone(),
                        attempt: 0,
                    });
                }
                Ok(CallToolResult::structured(json!({
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
                })))
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

        match self.store.get(&params.id).await {
            Ok(memory) => {
                // Implicit salience bump on direct retrieval (fire-and-forget, not on search results)
                if let Some(ref pg_store) = self.pg_store {
                    let store = pg_store.clone();
                    let id = params.id.clone();
                    tokio::spawn(async move {
                        if let Err(e) = store.touch_salience(&id).await {
                            tracing::warn!("Failed to touch salience for {}: {}", id, e);
                        }
                    });
                }
                Ok(CallToolResult::structured(json!({
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
                })))
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
        };

        match self.store.update(&params.id, input).await {
            Ok(memory) => {
                // Re-embed when content or tags change (tags are part of the embedding text)
                if content_changed || tags_changed {
                    if let Some(ref pipeline) = self.pipeline {
                        let text = crate::embedding::build_embedding_text(&memory.content, &memory.tags);
                        pipeline.enqueue(EmbeddingJob {
                            memory_id: memory.id.clone(),
                            text,
                            attempt: 0,
                        });
                    }
                }
                // Re-extract when content changes (extraction is content-only, not tags)
                if content_changed {
                    if let Some(ref extraction_pipeline) = self.extraction_pipeline {
                        // Reset extraction status to pending, then enqueue
                        if let Some(ref pg_store) = self.pg_store {
                            let store = pg_store.clone();
                            let id = memory.id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = store.update_extraction_status(&id, "pending").await {
                                    tracing::warn!("Failed to reset extraction status for {}: {}", id, e);
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
                Ok(CallToolResult::structured(json!({
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
                })))
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

        match self.store.delete(&params.id).await {
            Ok(()) => Ok(CallToolResult::structured(json!({
                "deleted": true,
                "id": params.id,
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
        };

        match self.store.list(filter).await {
            Ok(result) => {
                let memories: Vec<serde_json::Value> = result
                    .memories
                    .iter()
                    .map(|m| {
                        json!({
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
                        })
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
Params: query (required), limit (1-100, default 20), fields (array of field names for projection), \
min_salience (0.0-1.0, server-side quality filter), cursor (pagination token), \
tags (array, all must match), audience, created_after/created_before (ISO-8601), \
bm25_weight/vector_weight/symbolic_weight (0-1).\n\
Default output: {\"memories\": [{\"id\": \"uuid\", \"content\": \"text\", \"type_hint\": \"fact\", \
\"source\": \"default\", \"tags\": [\"t1\"], \"created_at\": \"ISO8601\", \"updated_at\": \"ISO8601\", \
\"access_count\": 0, \"relevance_score\": 0.85, \"match_source\": \"hybrid\", \
\"rrf_score\": 0.031, \"actor\": null, \"actor_type\": \"agent\", \"audience\": \"global\"}], \
\"total_results\": 1, \"query\": \"...\", \"next_cursor\": \"...\", \"has_more\": false}.\n\
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

        // 4. Query Intelligence: expansion (if enabled)
        let qi_start = Instant::now();
        let qi_budget = Duration::from_millis(self.qi_config.latency_budget_ms);

        let (search_query, qi_time_range) = if let Some(ref provider) = self.qi_expansion_provider {
            let expansion_budget = qi_budget * 6 / 10; // 60% for expansion
            match tokio::time::timeout(expansion_budget, provider.expand(&params.query)).await {
                Ok(Ok(expanded)) => {
                    tracing::info!(
                        variants = expanded.variants.len(),
                        has_time_range = expanded.time_range.is_some(),
                        "Query expanded"
                    );
                    // Use first variant as the search query (best formulation)
                    let best_query = expanded.variants.into_iter().next().unwrap_or_else(|| params.query.clone());
                    (best_query, expanded.time_range)
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "Query expansion failed, using original query");
                    (params.query.clone(), None)
                }
                Err(_) => {
                    tracing::warn!(elapsed_ms = ?qi_start.elapsed().as_millis(), "Query expansion timed out, using original query");
                    (params.query.clone(), None)
                }
            }
        } else {
            // No LLM expansion — try deterministic temporal fallback
            let time_range = parse_temporal_hint(&params.query, Utc::now());
            (params.query.clone(), time_range)
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
        let raw_hits = match pg_store.hybrid_search(
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
        ).await {
            Ok(hits) => hits,
            Err(e) => return Ok(store_error_to_result(e)),
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
        if let Some(ref time_range) = qi_time_range {
            for hit in &mut scored_hits {
                let created = hit.memory.created_at;
                let in_range = match (time_range.after, time_range.before) {
                    (Some(after), Some(before)) => created >= after && created <= before,
                    (Some(after), None) => created >= after,
                    (None, Some(before)) => created <= before,
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

        // Verify memory exists
        match self.store.get(&params.id).await {
            Err(MemcpError::NotFound { .. }) => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!("Memory not found: {}", params.id),
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

        match pg_store.reinforce_salience(&params.id, rating).await {
            Ok(row) => Ok(CallToolResult::structured(json!({
                "id": params.id,
                "stability": row.stability,
                "reinforcement_count": row.reinforcement_count,
                "message": format!(
                    "Memory reinforced. Stability: {:.1} days, reinforcements: {}",
                    row.stability, row.reinforcement_count
                )
            }))),
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

        let pg_store = match &self.pg_store {
            Some(s) => s,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "Feedback requires PostgreSQL backend"
                })));
            }
        };

        match pg_store.apply_feedback(&params.id, &params.signal).await {
            Ok(()) => Ok(CallToolResult::structured(json!({ "ok": true }))),
            Err(e) => Ok(store_error_to_result(e)),
        }
    }

    #[tool(description = "Recall relevant memories for automatic context injection. \
Returns up to N memories above relevance threshold, excluding already-recalled memories for this session. \
Session-scoped dedup prevents re-injection within a conversation. \
Returns {\"session_id\": \"...\", \"count\": N, \"memories\": [{\"memory_id\": \"uuid\", \"content\": \"...\", \"relevance\": 0.84}]}. \
Callable from code_execution_20260120 sandboxes.")]
    async fn recall_memory(
        &self,
        Parameters(params): Parameters<RecallMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            tool = "recall_memory",
            session_id = ?params.session_id,
            reset = params.reset.unwrap_or(false),
            "Tool called"
        );

        if params.query.trim().is_empty() {
            return Ok(CallToolResult::structured_error(json!({
                "isError": true,
                "error": "Field 'query' is required and cannot be empty",
                "field": "query"
            })));
        }

        // Embed query using inline embedding provider (same as search_memory).
        let embedding_provider = match &self.embedding_provider {
            Some(p) => p,
            None => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": "recall_memory requires an embedding provider — start with 'memcp serve' which loads the provider on startup"
                })));
            }
        };

        let query_embedding = match embedding_provider.embed(&params.query).await {
            Ok(emb) => emb,
            Err(e) => {
                return Ok(CallToolResult::structured_error(json!({
                    "isError": true,
                    "error": format!("Embedding failed: {}", e)
                })));
            }
        };

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

        // Create RecallEngine and execute.
        let engine = crate::recall::RecallEngine::new(
            pg_store,
            self.recall_config.clone(),
            self.extraction_enabled,
        );

        let result = engine
            .recall(&query_embedding, params.session_id, params.reset.unwrap_or(false))
            .await;

        match result {
            Ok(r) => Ok(CallToolResult::structured(json!({
                "session_id": r.session_id,
                "count": r.count,
                "memories": r.memories,
            }))),
            Err(e) => Ok(store_error_to_result(e)),
        }
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
                "Memory server for AI agents. Tools: store_memory, get_memory, search_memory, update_memory, delete_memory, bulk_delete_memories, list_memories, reinforce_memory, health_check.\n\nSearch uses keyword + semantic matching ranked by salience (recency, access frequency, relevance, reinforcement). Use list_memories to browse by filters. Reinforcement uses spaced repetition — faded memories get stronger boosts.\n\nDefaults: type_hint=\"fact\", source=\"default\", actor_type=\"agent\", audience=\"global\". Weights: 0=disable leg, 1=default, >1=emphasize.".to_string()
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
