//! Kimi (Moonshot) reasoning adapter — stub in plan 01.
//!
//! Real implementation lands in Phase 25 Plan 02. The stub returns
//! `NotConfigured` from both `new()` and `generate()` so the factory is
//! wireable without the adapter body being ready.

use async_trait::async_trait;

use super::{
    ProviderCredentials, ReasoningError, ReasoningProvider, ReasoningRequest, ReasoningResponse,
};
use crate::config::ProfileConfig;

pub struct KimiReasoningProvider {
    model: String,
}

impl KimiReasoningProvider {
    pub fn new(
        _profile: &ProfileConfig,
        _creds: ProviderCredentials,
    ) -> Result<Self, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "kimi adapter — plan 02".into(),
        ))
    }
}

#[async_trait]
impl ReasoningProvider for KimiReasoningProvider {
    async fn generate(
        &self,
        _req: &ReasoningRequest,
    ) -> Result<ReasoningResponse, ReasoningError> {
        Err(ReasoningError::NotConfigured(
            "kimi adapter — plan 02".into(),
        ))
    }
    fn model_name(&self) -> &str {
        &self.model
    }
}
