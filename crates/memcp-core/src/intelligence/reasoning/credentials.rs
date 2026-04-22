//! Provider credentials — populated by the transport layer (Phase 25 D-09).
//!
//! The `ReasoningProvider` trait NEVER reads env or headers directly. Transport
//! layer (HTTP middleware, daemon boot) constructs `ProviderCredentials` and
//! passes it into the factory. Keeps the Pro (env) / BYOK (header) routing
//! choice at the security boundary instead of smeared across every adapter.

use axum::http::HeaderMap;

use super::ReasoningError;

/// Provider credentials — populated by the transport layer (D-09).
///
/// The trait NEVER reads env or headers directly. Adapters receive this struct
/// via the factory and use [`require_api_key`] when the provider demands one
/// (Ollama for example leaves `api_key = None`).
#[derive(Clone, Debug, Default)]
pub struct ProviderCredentials {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

impl ProviderCredentials {
    /// Pro path: read from `MEMCP_REASONING__<PROVIDER>_API_KEY` env var.
    ///
    /// Base URL is also read from env (`MEMCP_REASONING__<PROVIDER>_BASE_URL`)
    /// but adapters supply sensible defaults if the env var is unset
    /// (Kimi: `https://api.moonshot.ai/v1`, OpenAI: `https://api.openai.com/v1`,
    /// Ollama: `http://localhost:11434`).
    pub fn from_env(provider: &str) -> Self {
        let key_var = format!("MEMCP_REASONING__{}_API_KEY", provider.to_uppercase());
        let base_var = format!("MEMCP_REASONING__{}_BASE_URL", provider.to_uppercase());
        ProviderCredentials {
            api_key: std::env::var(&key_var).ok().filter(|s| !s.is_empty()),
            base_url: std::env::var(&base_var).ok().filter(|s| !s.is_empty()),
        }
    }

    /// BYOK path: parse `x-reasoning-api-key` header.
    ///
    /// Caller separately validates `x-reasoning-provider` matches the D-01
    /// registry. Returns `None` when header is absent.
    ///
    /// `base_url` is NEVER caller-supplied (SSRF mitigation per 25-RESEARCH
    /// §Security Domain). Adapter defaults always win on the BYOK path.
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let key = headers
            .get("x-reasoning-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        Some(ProviderCredentials {
            api_key: Some(key),
            base_url: None,
        })
    }

    /// Extract the API key or return a `NotConfigured` error naming the provider.
    pub fn require_api_key(&self, provider: &str) -> Result<&str, ReasoningError> {
        self.api_key.as_deref().ok_or_else(|| {
            ReasoningError::NotConfigured(format!(
                "{provider} API key required — set MEMCP_REASONING__{}_API_KEY (Pro) or x-reasoning-api-key header (BYOK)",
                provider.to_uppercase()
            ))
        })
    }
}
