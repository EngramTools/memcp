//! OpenAI reasoning adapter — stub in plan 01.
//!
//! Real implementation lands in Phase 25 Plan 03.

use async_trait::async_trait;

use super::{
    ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest, ReasoningResponse,
};
use crate::config::ProfileConfig;

pub struct OpenAIReasoningProvider {
    model: String,
}

impl OpenAIReasoningProvider {
    pub fn new(
        _profile: &ProfileConfig,
        _creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "openai adapter — plan 03".into(),
        ))
    }
}

#[async_trait]
impl ReasoningProvider for OpenAIReasoningProvider {
    async fn generate(
        &self,
        _req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "openai adapter — plan 03".into(),
        ))
    }
    fn model_name(&self) -> &str {
        &self.model
    }
}
