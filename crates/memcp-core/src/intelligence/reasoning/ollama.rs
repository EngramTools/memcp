//! Ollama reasoning adapter — stub in plan 01.
//!
//! Real implementation lands in Phase 25 Plan 04.

use async_trait::async_trait;

use super::{
    ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest, ReasoningResponse,
};
use crate::config::ProfileConfig;

pub struct OllamaReasoningProvider {
    model: String,
}

impl OllamaReasoningProvider {
    pub fn new(
        _profile: &ProfileConfig,
        _creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "ollama adapter — plan 04".into(),
        ))
    }
}

#[async_trait]
impl ReasoningProvider for OllamaReasoningProvider {
    async fn generate(
        &self,
        _req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "ollama adapter — plan 04".into(),
        ))
    }
    fn model_name(&self) -> &str {
        &self.model
    }
}
