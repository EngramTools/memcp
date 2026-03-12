//! Request and response types for the HTTP API.
//!
//! All request structs use `#[serde(default)]` on optional fields for forward
//! compatibility — unknown fields are ignored, missing optional fields use defaults.

use serde::{Deserialize, Serialize};

/// Request body for POST /v1/recall.
#[derive(Deserialize)]
pub struct RecallRequest {
    /// Query string. None or empty string = queryless recall (salience-ranked, no embedding).
    #[serde(default)]
    pub query: Option<String>,
    /// Session identifier for dedup. Auto-generated if absent.
    pub session_id: Option<String>,
    /// When true, injects project summary + preamble (first-message context injection).
    #[serde(default)]
    pub first: bool,
    /// When true, clears session recall history before querying.
    #[serde(default)]
    pub reset: bool,
    /// Project scope. Returns project-scoped + global memories.
    #[serde(alias = "workspace")]
    pub project: Option<String>,
    /// Max memories to return (overrides config.recall.max_memories for queryless path).
    pub limit: Option<usize>,
    /// Tag affinity boost: memories sharing these tags get a relevance bonus.
    /// Prefix matching: "channel:" boosts all "channel:*" tags.
    #[serde(default)]
    pub boost_tags: Vec<String>,
    /// Content detail level: 0=abstract, 1=overview, 2=full content (default).
    /// Falls back gracefully if tier unavailable.
    #[serde(default = "default_depth")]
    pub depth: u8,
}

/// Request body for POST /v1/search.
#[derive(Deserialize)]
pub struct SearchRequest {
    /// Required search query.
    pub query: String,
    /// Max results to return (default: 10).
    #[serde(default = "default_search_limit")]
    pub limit: u32,
    /// Filter by tags (memories must have ALL specified tags).
    pub tags: Option<Vec<String>>,
    /// Filter by source(s).
    pub source: Option<Vec<String>>,
    /// Filter by type_hint (fact, summary, decision, etc).
    pub type_hint: Option<String>,
    /// Filter by audience scope.
    pub audience: Option<String>,
    /// Project scope filter.
    #[serde(alias = "workspace")]
    pub project: Option<String>,
    /// Minimum composite salience score (0.0–1.0).
    pub min_salience: Option<f64>,
    /// Field projection: only return these fields in each result object.
    pub fields: Option<Vec<String>>,
    /// Pagination cursor from previous page's next_cursor field.
    pub cursor: Option<String>,
    /// Content detail level: 0=abstract, 1=overview, 2=full content (default).
    /// Falls back gracefully if tier unavailable.
    #[serde(default = "default_depth")]
    pub depth: u8,
}

fn default_search_limit() -> u32 {
    10
}

fn default_depth() -> u8 {
    2
}

/// Redaction metadata included in store responses when content was redacted.
#[derive(Serialize, Clone, Debug)]
pub struct RedactionInfo {
    /// Number of individual redactions applied.
    pub count: usize,
    /// Unique categories of redacted content (e.g., "aws_key", "github_pat").
    pub categories: Vec<String>,
}

/// Request body for POST /v1/store.
#[derive(Deserialize)]
pub struct StoreRequest {
    /// Required memory content.
    pub content: String,
    /// Memory type (fact, preference, instruction, decision, summary).
    #[serde(default = "default_type_hint")]
    pub type_hint: String,
    /// Source identifier for provenance.
    #[serde(default = "default_source")]
    pub source: String,
    /// Tags for categorization and retrieval.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Actor identifier (agent name, user, etc).
    pub actor: Option<String>,
    /// Actor type (agent, user, system).
    #[serde(default = "default_actor_type")]
    pub actor_type: String,
    /// Audience scope (global, agent, user).
    #[serde(default = "default_audience")]
    pub audience: String,
    /// Idempotency key for dedup within the configured window.
    pub idempotency_key: Option<String>,
    /// When true, blocks until embedding completes before responding.
    #[serde(default)]
    pub wait: bool,
    /// Project scope for this memory.
    #[serde(alias = "workspace")]
    pub project: Option<String>,
    /// Trust level 0.0-1.0. Omit to let memcp infer from source/actor_type.
    #[serde(default)]
    pub trust_level: Option<f32>,
    /// Session identifier for grouping memories by conversation.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Agent's role when creating this memory (e.g., coder, reviewer, planner).
    #[serde(default)]
    pub agent_role: Option<String>,
    /// How this memory was created: "session_summary", "explicit_store", "annotation", "import".
    #[serde(default)]
    pub write_path: Option<String>,
    /// When true, bypasses secret/PII redaction. Default: false (redaction enabled).
    #[serde(default)]
    pub skip_redaction: bool,
}

fn default_type_hint() -> String {
    "fact".to_string()
}
fn default_source() -> String {
    "api".to_string()
}
fn default_actor_type() -> String {
    "agent".to_string()
}
fn default_audience() -> String {
    "global".to_string()
}

/// Request body for POST /v1/annotate.
#[derive(Deserialize)]
pub struct AnnotateRequest {
    /// Required: ID of the memory to annotate.
    pub id: String,
    /// Tags to append (merged with existing tags, deduplicated).
    pub tags: Option<Vec<String>>,
    /// Tags to replace (replaces all existing tags).
    pub replace_tags: Option<Vec<String>>,
    /// Salience override. Absolute ("3.0") or multiplier ("1.5x").
    pub salience: Option<String>,
}

/// Request body for POST /v1/update.
#[derive(Deserialize)]
pub struct UpdateRequest {
    /// Required: ID of the memory to update.
    pub id: String,
    /// New content (triggers re-embedding when changed).
    pub content: Option<String>,
    /// New type_hint.
    pub type_hint: Option<String>,
    /// New source.
    pub source: Option<String>,
    /// New tags (replaces all existing tags).
    pub tags: Option<Vec<String>>,
    /// Trust level override 0.0-1.0. Updates the stored trust level with JSONB audit trail.
    pub trust_level: Option<f32>,
}

/// Shared error body serializer.
#[derive(Serialize)]
pub struct ErrorBody {
    pub error: String,
}

/// Build a JSON error body for handler returns.
pub fn error_json(msg: &str) -> serde_json::Value {
    serde_json::json!({"error": msg})
}
