//! OpenAI-compatible query intelligence provider
//!
//! Calls any OpenAI-compatible Chat Completions API with json_object response format.
//! The base_url is configurable — supports OpenAI, Kimi Code API, and any compatible endpoint.
//! Requires an API key.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    DecomposedQuery, ExpandedQuery, QueryIntelligenceError, QueryIntelligenceProvider,
    RankedCandidate, RankedResult, TimeRange, build_decomposition_prompt, build_expansion_prompt,
    build_reranking_prompt,
};

// --- HTTP request/response structs (local — mirrors extraction/openai.rs pattern) ---

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    response_format: ResponseFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

// --- JSON output structs ---

/// Parsed query expansion output from LLM
#[derive(Deserialize)]
struct ExpandedQueryOutput {
    #[serde(default)]
    variants: Vec<String>,
    time_range: Option<TimeRangeOutput>,
}

/// Parsed query decomposition output from LLM
#[derive(Deserialize)]
struct DecomposedQueryOutput {
    #[serde(default)]
    is_multi_faceted: bool,
    #[serde(default)]
    sub_queries: Vec<String>,
    #[serde(default)]
    variants: Vec<String>,
    time_range: Option<TimeRangeOutput>,
}

#[derive(Deserialize)]
struct TimeRangeOutput {
    after: Option<String>,
    before: Option<String>,
}

// --- Provider ---

/// OpenAI-compatible query intelligence provider.
///
/// Uses the chat completions API with json_object response format.
/// base_url is configurable — not hardcoded — enabling Kimi Code API and other
/// OpenAI-compatible endpoints in addition to api.openai.com.
pub struct OpenAIQueryIntelligenceProvider {
    client: reqwest::Client,
    /// Configurable base URL — supports Kimi and other OpenAI-compatible APIs
    base_url: String,
    api_key: String,
    model: String,
}

impl OpenAIQueryIntelligenceProvider {
    /// Create a new OpenAIQueryIntelligenceProvider.
    ///
    /// # Arguments
    /// * `base_url` - API base URL (e.g., "https://api.openai.com/v1" or Kimi endpoint)
    /// * `api_key` - API key (must be non-empty)
    /// * `model` - Model name (e.g., "gpt-4o-mini")
    ///
    /// # Errors
    /// Returns `QueryIntelligenceError::NotConfigured` if api_key is empty.
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
    ) -> Result<Self, QueryIntelligenceError> {
        if api_key.trim().is_empty() {
            return Err(QueryIntelligenceError::NotConfigured(
                "OpenAI API key is required when using the openai query intelligence provider. \
                 Set the api_key in QueryIntelligenceConfig"
                    .to_string(),
            ));
        }

        Ok(OpenAIQueryIntelligenceProvider {
            client: reqwest::Client::new(),
            base_url,
            api_key,
            model,
        })
    }

    /// POST to {base_url}/chat/completions with json_object response format.
    async fn chat(&self, prompt: String) -> Result<String, QueryIntelligenceError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            response_format: ResponseFormat {
                format_type: "json_object".to_string(),
            },
            max_tokens: Some(2048),
        };

        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                QueryIntelligenceError::Generation(format!("HTTP request failed: {}", e))
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(QueryIntelligenceError::Api { status, message: body });
        }

        let chat_response: ChatResponse = response.json().await.map_err(|e| {
            QueryIntelligenceError::Generation(format!("Failed to parse OpenAI response: {}", e))
        })?;

        let content = chat_response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| {
                QueryIntelligenceError::Generation(
                    "OpenAI returned empty choices list".to_string(),
                )
            })?;

        Ok(content)
    }

    /// Shared expansion logic used by both expand() and expand_with_date().
    async fn expand_internal(
        &self,
        query: &str,
        current_date: &str,
    ) -> Result<ExpandedQuery, QueryIntelligenceError> {
        let prompt = build_expansion_prompt(query, current_date);

        let content = self.chat(prompt).await?;

        let output: ExpandedQueryOutput = serde_json::from_str(&content).map_err(|e| {
            QueryIntelligenceError::Generation(format!(
                "Failed to parse expansion JSON from model output: {} (content: {})",
                e, &content
            ))
        })?;

        if output.variants.is_empty() {
            return Err(QueryIntelligenceError::Generation(
                format!("LLM returned no query variants (raw content: {})", &content),
            ));
        }

        let time_range = output.time_range.map(|tr| TimeRange {
            after: parse_datetime_opt(tr.after),
            before: parse_datetime_opt(tr.before),
        });

        Ok(ExpandedQuery {
            variants: output.variants,
            time_range,
        })
    }
}

/// Parse an optional RFC-3339 string to DateTime<Utc>, returning None on failure.
fn parse_datetime_opt(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|raw| {
        DateTime::parse_from_rfc3339(&raw)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
}

#[async_trait]
impl QueryIntelligenceProvider for OpenAIQueryIntelligenceProvider {
    async fn decompose(&self, query: &str) -> Result<DecomposedQuery, QueryIntelligenceError> {
        let current_date = Utc::now().format("%Y-%m-%d").to_string();
        let prompt = build_decomposition_prompt(query, &current_date);

        let content = match self.chat(prompt).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "OpenAI decomposition request failed, using raw query");
                return Ok(DecomposedQuery {
                    is_multi_faceted: false,
                    sub_queries: vec![],
                    variants: vec![query.to_string()],
                    time_range: None,
                });
            }
        };

        let output: DecomposedQueryOutput = match serde_json::from_str(&content) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    content = %content,
                    "Failed to parse decomposition JSON, using raw query"
                );
                return Ok(DecomposedQuery {
                    is_multi_faceted: false,
                    sub_queries: vec![],
                    variants: vec![query.to_string()],
                    time_range: None,
                });
            }
        };

        let is_multi = output.is_multi_faceted && !output.sub_queries.is_empty();
        tracing::debug!(
            is_multi_faceted = is_multi,
            sub_query_count = output.sub_queries.len(),
            variant_count = output.variants.len(),
            "Query decomposed"
        );

        let time_range = output.time_range.map(|tr| TimeRange {
            after: parse_datetime_opt(tr.after),
            before: parse_datetime_opt(tr.before),
        });

        Ok(DecomposedQuery {
            is_multi_faceted: is_multi,
            sub_queries: if is_multi { output.sub_queries } else { vec![] },
            variants: if is_multi {
                vec![]
            } else if output.variants.is_empty() {
                vec![query.to_string()]
            } else {
                output.variants
            },
            time_range,
        })
    }

    async fn expand(&self, query: &str) -> Result<ExpandedQuery, QueryIntelligenceError> {
        let current_date = Utc::now().format("%Y-%m-%d").to_string();
        self.expand_internal(query, &current_date).await
    }

    async fn expand_with_date(
        &self,
        query: &str,
        reference_date: &str,
    ) -> Result<ExpandedQuery, QueryIntelligenceError> {
        self.expand_internal(query, reference_date).await
    }

    async fn rerank(
        &self,
        query: &str,
        candidates: &[RankedCandidate],
    ) -> Result<Vec<RankedResult>, QueryIntelligenceError> {
        // Send 1-indexed integer IDs to the model instead of UUIDs.
        // This drastically shrinks the output (~50 tokens vs ~800 for 20 UUIDs)
        // and eliminates truncation failures.
        let idx_to_real_id: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();

        let candidates_json = {
            let arr: Vec<serde_json::Value> = candidates
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    serde_json::json!({
                        "id": i + 1,
                        "content": c.content,
                        "rank": c.current_rank
                    })
                })
                .collect();
            serde_json::to_string(&arr).map_err(|e| {
                QueryIntelligenceError::Generation(format!(
                    "Failed to serialize candidates: {}",
                    e
                ))
            })?
        };

        let prompt = build_reranking_prompt(query, &candidates_json);
        let content = self.chat(prompt).await?;

        // Parse ranked_ids — model returns integer indices as strings or numbers.
        let raw: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            QueryIntelligenceError::Generation(format!(
                "Failed to parse rerank JSON from model output: {} (content: {})",
                e, &content
            ))
        })?;

        let ranked_ids = raw
            .get("ranked_ids")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                QueryIntelligenceError::Generation(format!(
                    "Missing ranked_ids array in model output: {}",
                    &content
                ))
            })?;

        // Map integer indices back to real IDs
        let results: Vec<RankedResult> = ranked_ids
            .iter()
            .filter_map(|v| {
                let idx = v
                    .as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))?;
                let zero_idx = (idx as usize).checked_sub(1)?;
                idx_to_real_id.get(zero_idx).map(|&real_id| real_id.to_string())
            })
            .enumerate()
            .map(|(rank, id)| RankedResult {
                id,
                llm_rank: rank + 1,
            })
            .collect();

        Ok(results)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
