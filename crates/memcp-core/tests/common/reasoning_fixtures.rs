//! Shared mock providers for runner tests (plan 25-06).
//!
//! Every integration test file that needs these fixtures declares
//! `mod common { pub mod reasoning_fixtures; }` at the top and imports
//! the items it needs. Because each Rust integration test is its own
//! crate, the `dead_code` lint fires for unused items per-crate — silenced
//! at module scope below.

#![allow(dead_code)]

use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use memcp::errors::MemcpError;
use memcp::intelligence::reasoning::{
    AgentCallerContext, ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest,
    ReasoningResponse, ToolCall,
};
use memcp::storage::store::{
    CreateMemory, ListFilter, ListResult, Memory, MemoryStore, StoreOutcome, UpdateMemory,
};

/// Injects a deterministic sequence of canned responses. Each `generate()`
/// call pops the next response from the queue; panics if queue is empty
/// (signals a test bug — the test expected more turns than it queued).
#[derive(Clone)]
pub struct MockReasoningProvider {
    pub queue: Arc<Mutex<Vec<Result<ReasoningResponse, ReasoningError>>>>,
    pub model: String,
}

impl MockReasoningProvider {
    pub fn new(
        responses: Vec<Result<ReasoningResponse, ReasoningError>>,
        model: &str,
    ) -> Self {
        // Reverse so `pop()` yields responses in declared order.
        Self {
            queue: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
            model: model.to_string(),
        }
    }
}

#[async_trait]
impl ReasoningProvider for MockReasoningProvider {
    async fn generate(
        &self,
        _req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        self.queue.lock().unwrap().pop().unwrap_or_else(|| {
            panic!("MockReasoningProvider: queue exhausted — test expected more generate() calls")
        })
    }
    fn model_name(&self) -> &str {
        &self.model
    }
}

/// Provider that sleeps for `delay` before returning a single queued response.
/// Used to exercise the per-turn `tokio::time::timeout` branch.
#[derive(Clone)]
pub struct SlowMockProvider {
    pub delay: std::time::Duration,
    pub response: Arc<Mutex<Option<Result<ReasoningResponse, ReasoningError>>>>,
}

#[async_trait]
impl ReasoningProvider for SlowMockProvider {
    async fn generate(
        &self,
        _req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        tokio::time::sleep(self.delay).await;
        self.response
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(ReasoningError::Transport("no response".into())))
    }
    fn model_name(&self) -> &str {
        "slow-mock"
    }
}

/// Captures `req.max_tokens` from every `generate()` call for
/// the `test_max_tokens_bounded` budget test.
#[derive(Clone)]
pub struct RecordingMockProvider {
    pub captured_max_tokens: Arc<Mutex<Vec<u32>>>,
    pub responses: Arc<Mutex<Vec<Result<ReasoningResponse, ReasoningError>>>>,
}

impl RecordingMockProvider {
    pub fn new(responses: Vec<Result<ReasoningResponse, ReasoningError>>) -> Self {
        Self {
            captured_max_tokens: Arc::new(Mutex::new(vec![])),
            responses: Arc::new(Mutex::new(responses.into_iter().rev().collect())),
        }
    }
}

#[async_trait]
impl ReasoningProvider for RecordingMockProvider {
    async fn generate(
        &self,
        req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        self.captured_max_tokens.lock().unwrap().push(req.max_tokens);
        self.responses
            .lock()
            .unwrap()
            .pop()
            .unwrap_or(Err(ReasoningError::Transport("exhausted".into())))
    }
    fn model_name(&self) -> &str {
        "recording-mock"
    }
}

/// Build a ToolCall with a distinct id + arbitrary args. Used by termination
/// tests that need multiple DIFFERENT tool calls to avoid tripping the
/// repeated-call detector.
pub fn tc_call_with_args(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("call-{}", uuid::Uuid::new_v4()),
        name: name.to_string(),
        arguments: args,
    }
}

/// No-op MemoryStore used by runner tests that don't exercise the store.
///
/// All write/read methods return errors — the runner tests either pass
/// `tools=[]` (so tool_calls produce "unknown_tool" error results that keep
/// the loop running) or pass `memory_tools()` and rely on the dispatcher's
/// structured-error wrapping.
pub struct NullStore;

#[async_trait]
impl MemoryStore for NullStore {
    async fn store_with_outcome(
        &self,
        _: CreateMemory,
    ) -> Result<StoreOutcome, MemcpError> {
        Err(MemcpError::Internal("null-store".into()))
    }
    async fn get(&self, _: &str) -> Result<Memory, MemcpError> {
        Err(MemcpError::Internal("null-store".into()))
    }
    async fn update(&self, _: &str, _: UpdateMemory) -> Result<Memory, MemcpError> {
        Err(MemcpError::Internal("null-store".into()))
    }
    async fn delete(&self, _: &str) -> Result<(), MemcpError> {
        Ok(())
    }
    async fn list(&self, _: ListFilter) -> Result<ListResult, MemcpError> {
        Ok(ListResult {
            memories: vec![],
            next_cursor: None,
        })
    }
    async fn count_matching(&self, _: &ListFilter) -> Result<u64, MemcpError> {
        Ok(0)
    }
    async fn delete_matching(&self, _: &ListFilter) -> Result<u64, MemcpError> {
        Ok(0)
    }
    async fn touch(&self, _: &str) -> Result<(), MemcpError> {
        Ok(())
    }
}

/// Build a no-op AgentCallerContext suitable for runner termination/budget tests.
pub fn noop_ctx() -> AgentCallerContext {
    AgentCallerContext {
        store: Arc::new(NullStore),
        creds: ProviderCredentials::default(),
        run_id: "test-run".into(),
        final_selection: Mutex::new(HashSet::new()),
        read_but_discarded: Mutex::new(HashSet::new()),
        tombstoned: Mutex::new(HashSet::new()),
    }
}
