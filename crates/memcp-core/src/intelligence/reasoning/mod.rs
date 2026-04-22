//! Reasoning agent framework: trait + unified types + factory.
//!
//! Phase 25: ships 3 adapters (Kimi, OpenAI, Ollama). Phase 25.1 extends.
//! Per Phase 25 D-04: adapters translate in/out of their native tool-call shape;
//! callers (P26 dreaming, P27 agentic retrieval) see ONLY the unified types below.

pub mod credentials;
pub mod kimi; // stub in plan 01 — impl lands in plan 02
pub mod ollama; // stub in plan 01 — impl lands in plan 04
pub mod openai; // stub in plan 01 — impl lands in plan 03
pub mod runner; // plan 06 — iteration-loop runner (REAS-07 + REAS-08)
pub mod tools; // plan 05 — 6 memory tools + dispatch_tool

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

pub use credentials::ProviderCredentials;
pub use runner::{run_agent, run_agent_with_provider, run_agent_with_provider_and_timeout};
pub use tools::{dispatch_tool, memory_tools, validate_tool_schemas};

use crate::config::ProfileConfig;
use crate::errors::MemcpError;

// ─── Unified types (D-04) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema (draft-07) for the tool's arguments.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned or adapter-synthesized ID (Kimi: "search:0",
    /// OpenAI: "call_xyz", Ollama: synthesized at adapter boundary).
    pub id: String,
    pub name: String,
    /// ALWAYS a parsed Value — adapters normalize stringified JSON
    /// (OpenAI/Kimi) at the translate_out boundary (RESEARCH Pitfall 1).
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl Message {
    pub fn assistant_with_tools(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Message::Assistant {
            content,
            tool_calls,
        }
    }
    pub fn tool_result(r: &ToolResult) -> Self {
        Message::Tool {
            tool_call_id: r.tool_call_id.clone(),
            content: r.content.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct ReasoningRequest {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone)]
pub struct ReasoningResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    /// finish_reason is logged for debugging only — NEVER used as loop
    /// terminator (RESEARCH Pitfall 3).
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AgentOutcome {
    Terminal {
        content: Option<String>,
        tokens_used: u32,
        iterations: u32,
    },
    BudgetExceeded {
        tokens_used: u32,
        iterations: u32,
    },
    MaxIterations {
        tokens_used: u32,
        iterations: u32,
    },
    RepeatedToolCall {
        tokens_used: u32,
        iterations: u32,
    },
}

/// Per-run context passed from caller (transport layer or P26/P27 daemon) to
/// `run_agent`. D-09: transport layer populates `creds`; the trait never reads
/// env or headers.
pub struct AgentCallerContext {
    pub store: Arc<dyn crate::storage::store::MemoryStore>,
    pub creds: ProviderCredentials,
    /// Unique per-invocation; used as `run_id` in `salience_audit_log` for
    /// idempotent rollback.
    pub run_id: String,
    pub final_selection: std::sync::Mutex<std::collections::HashSet<String>>,
    pub read_but_discarded: std::sync::Mutex<std::collections::HashSet<String>>,
    pub tombstoned: std::sync::Mutex<std::collections::HashSet<String>>,
}

// ─── Error type ───────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ReasoningError {
    #[error("Reasoning generation error: {0}")]
    Generation(String),

    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    #[error("Provider not configured: {0}")]
    NotConfigured(String),

    #[error("Transport: {0}")]
    Transport(String),

    #[error("Bad tool schema for {0}: {1}")]
    BadToolSchema(String, String),

    #[error("Repeated tool call detected after 3x same (name, args)")]
    RepeatedToolCall,

    #[error("Budget exceeded: {tokens} tokens")]
    BudgetExceeded { tokens: u32 },

    #[error("Max iterations ({0}) reached")]
    MaxIterations(u32),
}

impl From<ReasoningError> for MemcpError {
    fn from(e: ReasoningError) -> Self {
        MemcpError::Internal(e.to_string())
    }
}

/// REAS-10 salience hook — stub in plan 06, fully implemented in plan 07.
///
/// Plan 07 replaces this body with x1.3 / x0.9 / x0.1 writes against
/// `PostgresMemoryStore::apply_stability_boost`. Runner calls this
/// unconditionally at every exit point (Terminal, BudgetExceeded,
/// MaxIterations, RepeatedToolCall) so the salience side-effects land
/// regardless of how the loop terminated.
pub async fn apply_salience_side_effects(
    _ctx: &AgentCallerContext,
) -> Result<(), ReasoningError> {
    // Plan 07 will read ctx.final_selection / read_but_discarded / tombstoned
    // and apply boosts via MemoryStore::apply_stability_boost.
    Ok(())
}

// ─── Trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait ReasoningProvider: Send + Sync {
    async fn generate(
        &self,
        req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError>;
    fn model_name(&self) -> &str;
}

// ─── Factory ──────────────────────────────────────────────────────────

/// Build a provider from profile config + credentials.
/// Plans 02/03/04 replace the stub `new()` bodies with real adapters.
pub fn create_reasoning_provider(
    profile: &ProfileConfig,
    creds: ProviderCredentials,
) -> Result<Arc<dyn ReasoningProvider>, ReasoningError> {
    match profile.provider.as_str() {
        "kimi" => kimi::KimiReasoningProvider::new(profile, creds)
            .map(|p| Arc::new(p) as Arc<dyn ReasoningProvider>),
        "openai" => openai::OpenAIReasoningProvider::new(profile, creds)
            .map(|p| Arc::new(p) as Arc<dyn ReasoningProvider>),
        "ollama" => ollama::OllamaReasoningProvider::new(profile, creds)
            .map(|p| Arc::new(p) as Arc<dyn ReasoningProvider>),
        other => Err(ReasoningError::NotConfigured(format!(
            "unknown provider: {other}"
        ))),
    }
}
