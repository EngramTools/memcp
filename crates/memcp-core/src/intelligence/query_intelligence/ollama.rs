//! Ollama query intelligence provider
//!
//! Calls the Ollama /api/chat endpoint with structured JSON output schema.
//! Supports both query expansion (with temporal hint extraction) and candidate re-ranking.
//! No API key required — designed for self-hosted Ollama deployments.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    DecomposedQuery, ExpandedQuery, QueryIntelligenceError, QueryIntelligenceProvider,
    RankedCandidate, RankedResult, TimeRange, build_decomposition_prompt, build_expansion_prompt,
    build_explain_connections_prompt, build_reranking_prompt, decomposition_schema,
    explain_connections_schema, expansion_schema, reranking_schema,
};

// --- HTTP request/response structs (local — mirrors extraction/ollama.rs pattern) ---

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
    format: serde_json::Value,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
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

/// Parsed connection explanation output from LLM
#[derive(Deserialize)]
struct ExplainConnectionsOutput {
    #[serde(default)]
    explanations: Vec<String>,
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

/// Ollama-backed query intelligence provider.
///
/// Uses /api/chat with structured JSON output (format field) for both
/// query expansion and result re-ranking.
pub struct OllamaQueryIntelligenceProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl OllamaQueryIntelligenceProvider {
    /// Create a new OllamaQueryIntelligenceProvider.
    ///
    /// # Arguments
    /// * `base_url` - Ollama server base URL (e.g., "http://localhost:11434")
    /// * `model` - Model name (e.g., "llama3.2:3b")
    pub fn new(base_url: String, model: String) -> Self {
        OllamaQueryIntelligenceProvider {
            client: reqwest::Client::new(),
            base_url,
            model,
        }
    }

    /// POST to Ollama /api/chat with a given prompt and schema, return content string.
    async fn chat(
        &self,
        prompt: String,
        schema: serde_json::Value,
    ) -> Result<String, QueryIntelligenceError> {
        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: vec![OllamaMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            stream: false,
            options: OllamaOptions { temperature: 0.0 },
            format: schema,
        };

        let url = format!("{}/api/chat", self.base_url);

        let response = self
            .client
            .post(&url)
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

        let chat_response: OllamaChatResponse = response.json().await.map_err(|e| {
            QueryIntelligenceError::Generation(format!("Failed to parse Ollama response: {}", e))
        })?;

        Ok(chat_response.message.content)
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
impl QueryIntelligenceProvider for OllamaQueryIntelligenceProvider {
    async fn decompose(&self, query: &str) -> Result<DecomposedQuery, QueryIntelligenceError> {
        let current_date = Utc::now().format("%Y-%m-%d").to_string();
        let prompt = build_decomposition_prompt(query, &current_date);

        let content = match self.chat(prompt, decomposition_schema()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "Ollama decomposition request failed, using raw query");
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
        let prompt = build_expansion_prompt(query, &current_date);

        let content = self.chat(prompt, expansion_schema()).await?;

        let output: ExpandedQueryOutput = serde_json::from_str(&content).map_err(|e| {
            QueryIntelligenceError::Generation(format!(
                "Failed to parse expansion JSON from model output: {} (content: {})",
                e, &content
            ))
        })?;

        if output.variants.is_empty() {
            return Err(QueryIntelligenceError::Generation(
                "LLM returned no query variants".to_string(),
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

    async fn rerank(
        &self,
        query: &str,
        candidates: &[RankedCandidate],
    ) -> Result<Vec<RankedResult>, QueryIntelligenceError> {
        // Send 1-indexed integer IDs instead of UUIDs to keep output short.
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
        let content = self.chat(prompt, reranking_schema()).await?;

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

    async fn explain_connections(
        &self,
        query: &str,
        results: &[(&str, f64)],
    ) -> Result<Vec<String>, QueryIntelligenceError> {
        if results.is_empty() {
            return Ok(vec![]);
        }

        // Build numbered list: "1. [0.47] content snippet"
        let numbered_items: String = results
            .iter()
            .enumerate()
            .map(|(i, (content, sim))| {
                let snippet: String = content.chars().take(120).collect();
                format!("{}. [{:.2}] {}", i + 1, sim, snippet)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = build_explain_connections_prompt(query, &numbered_items);

        let content = match self.chat(prompt, explain_connections_schema()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "Ollama explain_connections request failed");
                return Ok(vec![]);
            }
        };

        let output: ExplainConnectionsOutput = match serde_json::from_str(&content) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    content = %content,
                    "Failed to parse explain_connections JSON"
                );
                return Ok(vec![]);
            }
        };

        Ok(output.explanations)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
