//! Query expansion and LLM-based re-ranking.
//!
//! QueryIntelligenceProvider trait with Ollama and OpenAI implementations.
//! Expansion generates alternative query terms; reranking scores search candidates.
//! Feeds from transport/server + transport/ipc into intelligence/search/.

/// Query intelligence provider trait and supporting types
///
/// Provides a pluggable interface for LLM-based query expansion and re-ranking.
/// Supports Ollama (local, default, no API key) and OpenAI-compatible APIs.
///
/// Both features are disabled by default — set expansion_enabled or reranking_enabled
/// in QueryIntelligenceConfig to opt in.

pub mod ollama;
pub mod openai;
pub mod temporal;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::errors::MemcpError;

/// Errors that can occur during query intelligence operations.
#[derive(Debug, Error)]
pub enum QueryIntelligenceError {
    /// Inference or JSON parse failure
    #[error("Query intelligence generation error: {0}")]
    Generation(String),

    /// API provider returned an HTTP error
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    /// Provider not configured (e.g., missing API key or model)
    #[error("Provider not configured: {0}")]
    NotConfigured(String),

    /// Operation exceeded latency budget
    #[error("Query intelligence timeout: {0}")]
    Timeout(String),
}

impl From<QueryIntelligenceError> for MemcpError {
    fn from(e: QueryIntelligenceError) -> Self {
        MemcpError::Internal(e.to_string())
    }
}

/// A time range filter derived from temporal hints in a query.
#[derive(Debug, Clone)]
pub struct TimeRange {
    /// Lower bound (inclusive): memories after this timestamp
    pub after: Option<DateTime<Utc>>,
    /// Upper bound (inclusive): memories before this timestamp
    pub before: Option<DateTime<Utc>>,
}

/// Result of expanding a query via LLM.
#[derive(Debug, Clone)]
pub struct ExpandedQuery {
    /// Alternative phrasings of the query (2–3 variants, may exclude original)
    pub variants: Vec<String>,
    /// Optional time range extracted from temporal hints in the query
    pub time_range: Option<TimeRange>,
}

/// A candidate memory for re-ranking.
#[derive(Debug, Clone)]
pub struct RankedCandidate {
    /// Unique memory ID
    pub id: String,
    /// Memory content (may be truncated per rerank_content_chars config)
    pub content: String,
    /// Current rank in the retrieval result list (1-indexed, lower = more relevant)
    pub current_rank: usize,
}

/// A re-ranked memory result from the LLM.
#[derive(Debug, Clone)]
pub struct RankedResult {
    /// Memory ID
    pub id: String,
    /// New rank assigned by LLM (1-indexed, lower = more relevant)
    pub llm_rank: usize,
}

/// Core trait for LLM-based query expansion and candidate re-ranking.
///
/// Implementations must be Send + Sync to support use in async contexts
/// and across thread boundaries (e.g., Arc<dyn QueryIntelligenceProvider>).
#[async_trait]
pub trait QueryIntelligenceProvider: Send + Sync {
    /// Expand a query into variants and extract any temporal hints.
    async fn expand(&self, query: &str) -> Result<ExpandedQuery, QueryIntelligenceError>;

    /// Decompose a query into sub-queries (multi-faceted) or variants (simple).
    ///
    /// Replaces expand() as the primary query analysis method. For multi-faceted queries
    /// (e.g., "What were the auth decisions and database patterns?"), returns focused
    /// sub-queries that can be searched independently and merged via rrf_fuse_multi().
    /// For simple queries, returns variant phrasings (same as expand()).
    ///
    /// Default implementation falls back to expand() for backward compatibility.
    async fn decompose(&self, query: &str) -> Result<DecomposedQuery, QueryIntelligenceError> {
        let expanded = self.expand(query).await?;
        Ok(DecomposedQuery {
            is_multi_faceted: false,
            sub_queries: vec![],
            variants: expanded.variants,
            time_range: expanded.time_range,
        })
    }

    /// Re-rank retrieved candidates, returning them in LLM-preferred order.
    async fn rerank(
        &self,
        query: &str,
        candidates: &[RankedCandidate],
    ) -> Result<Vec<RankedResult>, QueryIntelligenceError>;

    /// Return the model name identifier used by this provider.
    fn model_name(&self) -> &str;

    /// Explain why each discovered memory is an interesting/unexpected connection
    /// to the original query topic.
    ///
    /// Called after `discover_associations()` to generate per-result connection
    /// explanations. Returns one explanation string per result, in order.
    ///
    /// Fail-open: default implementation returns an empty vec (no explanations).
    /// Providers override this to provide LLM-generated explanations.
    async fn explain_connections(
        &self,
        _query: &str,
        _results: &[(&str, f64)],  // (content_snippet, similarity)
    ) -> Result<Vec<String>, QueryIntelligenceError> {
        Ok(vec![])
    }
}

/// Build the discover connection explanation prompt.
///
/// Asks the LLM to explain why each discovered memory represents an interesting
/// or unexpected connection to the query topic.
pub fn build_explain_connections_prompt(query: &str, numbered_items: &str) -> String {
    format!(
        "You are helping an AI agent understand unexpected memory connections.\n\
         The agent searched for '{query}' and found memories with moderate similarity \
         (0.3-0.7) — related but not directly about the topic.\n\n\
         For each memory below, write one short sentence (max 15 words) explaining \
         why it represents an interesting or unexpected connection to '{query}'.\n\n\
         Output only valid JSON: {{\"explanations\": [\"reason 1\", \"reason 2\", ...]}}\n\
         One explanation per memory, in the same order. No commentary.\n\n\
         Memories:\n{numbered_items}"
    )
}

/// JSON schema for connection explanation output.
pub fn explain_connections_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "explanations": {
                "type": "array",
                "items": { "type": "string" },
                "description": "One short explanation per discovered memory, in order"
            }
        },
        "required": ["explanations"]
    })
}

/// Build the query expansion prompt.
///
/// Instructs the LLM to act as an AI assistant searching its own memory bank,
/// generate 2–3 query variants, and extract any temporal hints.
pub fn build_expansion_prompt(query: &str, current_date: &str) -> String {
    format!(
        "You are helping an AI assistant search its own memory bank.\n\
         Today's date: {current_date}\n\n\
         Given the search query below, do two things:\n\
         1. Generate 2-3 alternative phrasings that would help retrieve relevant memories \
            (you may discard the original if a variant is clearly better).\n\
         2. If the query contains a temporal hint (e.g. 'last week', 'yesterday', \
            'after 2024-01-01'), extract it as a time range with ISO-8601 after/before fields.\n\n\
         Output only valid JSON. Do not add commentary.\n\
         Schema: {{\"variants\": [\"alt phrasing 1\", \"alt phrasing 2\"], \"time_range\": null}}\n\
         If a time range exists: {{\"variants\": [...], \"time_range\": {{\"after\": \"2024-01-01T00:00:00Z\", \"before\": null}}}}\n\n\
         Query: {query}"
    )
}

/// Build the re-ranking prompt.
///
/// Instructs the LLM to re-order candidate memories by relevance to the query.
/// Candidates use integer IDs (1, 2, 3, ...) to keep output short.
pub fn build_reranking_prompt(query: &str, candidates_json: &str) -> String {
    format!(
        "You are helping an AI assistant search its own memory bank.\n\
         Given the search query and a list of candidate memories below, \
         re-order the candidates from most relevant to least relevant.\n\n\
         Output only valid JSON: {{\"ranked_ids\": [3, 1, 2, ...]}}. \
         Use the integer IDs from the candidates. Include ALL IDs. No commentary.\n\n\
         Query: {query}\n\n\
         Candidates:\n{candidates_json}"
    )
}

/// JSON schema for expansion output.
///
/// `variants` is required; `time_range` is optional with optional after/before fields.
pub fn expansion_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "variants": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": 3,
                "description": "Alternative phrasings of the original query"
            },
            "time_range": {
                "type": "object",
                "properties": {
                    "after": {
                        "type": "string",
                        "description": "ISO-8601 datetime lower bound (inclusive)"
                    },
                    "before": {
                        "type": "string",
                        "description": "ISO-8601 datetime upper bound (inclusive)"
                    }
                }
            }
        },
        "required": ["variants"]
    })
}

/// Result of decomposing a query — either sub-queries (multi-faceted) or variants (simple).
///
/// When is_multi_faceted=true, sub_queries contains 2-4 focused queries (use rrf_fuse_multi).
/// When is_multi_faceted=false, variants contains 2-3 alternative phrasings (single search).
#[derive(Debug, Clone)]
pub struct DecomposedQuery {
    /// Whether the LLM determined this query has multiple facets
    pub is_multi_faceted: bool,
    /// Focused sub-queries for multi-faceted queries (2-4 items); empty when simple
    pub sub_queries: Vec<String>,
    /// Alternative phrasings for simple queries (2-3 items, same as ExpandedQuery.variants)
    pub variants: Vec<String>,
    /// Optional time range extracted from temporal hints
    pub time_range: Option<TimeRange>,
}

/// Build the query decomposition prompt.
///
/// Instructs the LLM to analyze whether the query asks about 2+ distinct topics
/// and either decompose into focused sub-queries or generate variant phrasings.
pub fn build_decomposition_prompt(query: &str, current_date: &str) -> String {
    format!(
        "You are helping an AI assistant search its own memory bank.\n\
         Today's date: {current_date}\n\n\
         Analyze this search query and do the following:\n\
         1. Determine if it is multi-faceted (asks about 2 or more distinct topics/concepts).\n\
         2. If MULTI-FACETED: break it into 2-4 focused sub-queries, each covering one topic.\n\
            Set is_multi_faceted=true, populate sub_queries, leave variants empty.\n\
         3. If SIMPLE (single topic): generate 2-3 alternative phrasings.\n\
            Set is_multi_faceted=false, populate variants, leave sub_queries empty.\n\
         4. If the query contains a temporal hint (e.g. 'last week', 'yesterday', \
            'after 2024-01-01'), extract it as a time range.\n\n\
         Output only valid JSON. Do not add commentary.\n\
         Multi-faceted example: {{\"is_multi_faceted\": true, \"sub_queries\": [\"auth decisions\", \"database patterns\"], \"variants\": [], \"time_range\": null}}\n\
         Simple example: {{\"is_multi_faceted\": false, \"sub_queries\": [], \"variants\": [\"how to configure redis\", \"redis setup guide\"], \"time_range\": null}}\n\n\
         Query: {query}"
    )
}

/// JSON schema for decomposition output.
///
/// is_multi_faceted is required; sub_queries and variants are conditionally populated.
pub fn decomposition_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "is_multi_faceted": {
                "type": "boolean",
                "description": "True if query asks about 2+ distinct topics requiring separate sub-queries"
            },
            "sub_queries": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": 4,
                "description": "Focused sub-queries for each topic (only when is_multi_faceted=true)"
            },
            "variants": {
                "type": "array",
                "items": { "type": "string" },
                "maxItems": 3,
                "description": "Alternative phrasings of the original query (only when is_multi_faceted=false)"
            },
            "time_range": {
                "type": "object",
                "properties": {
                    "after": {
                        "type": "string",
                        "description": "ISO-8601 datetime lower bound (inclusive)"
                    },
                    "before": {
                        "type": "string",
                        "description": "ISO-8601 datetime upper bound (inclusive)"
                    }
                }
            }
        },
        "required": ["is_multi_faceted"]
    })
}

/// JSON schema for re-ranking output.
///
/// `ranked_ids` must contain all candidate IDs, most relevant first.
pub fn reranking_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "ranked_ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "All candidate IDs ordered from most to least relevant"
            }
        },
        "required": ["ranked_ids"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decomposition_schema_valid() {
        let schema = decomposition_schema();
        // Must be an object type
        assert_eq!(schema["type"], "object");
        // Must have is_multi_faceted in required
        let required = schema["required"].as_array().expect("required must be array");
        assert!(required.iter().any(|v| v == "is_multi_faceted"), "is_multi_faceted must be required");
        // Must have all expected properties
        let props = &schema["properties"];
        assert!(props.get("is_multi_faceted").is_some(), "missing is_multi_faceted property");
        assert!(props.get("sub_queries").is_some(), "missing sub_queries property");
        assert!(props.get("variants").is_some(), "missing variants property");
        assert!(props.get("time_range").is_some(), "missing time_range property");
        // sub_queries maxItems must be 4
        assert_eq!(props["sub_queries"]["maxItems"], 4);
        // variants maxItems must be 3
        assert_eq!(props["variants"]["maxItems"], 3);
    }

    #[test]
    fn test_decomposed_query_from_expanded_wraps_correctly() {
        // Verify that DecomposedQuery built from ExpandedQuery data matches expected shape
        let variants = vec!["auth config".to_string(), "authentication setup".to_string()];
        let decomposed = DecomposedQuery {
            is_multi_faceted: false,
            sub_queries: vec![],
            variants: variants.clone(),
            time_range: None,
        };
        assert!(!decomposed.is_multi_faceted);
        assert!(decomposed.sub_queries.is_empty());
        assert_eq!(decomposed.variants, variants);
        assert!(decomposed.time_range.is_none());
    }

    #[test]
    fn test_decomposed_query_multi_faceted() {
        let decomposed = DecomposedQuery {
            is_multi_faceted: true,
            sub_queries: vec![
                "auth decisions".to_string(),
                "database patterns".to_string(),
            ],
            variants: vec![],
            time_range: None,
        };
        assert!(decomposed.is_multi_faceted);
        assert_eq!(decomposed.sub_queries.len(), 2);
        assert!(decomposed.variants.is_empty());
    }

    #[test]
    fn test_build_decomposition_prompt_contains_query() {
        let prompt = build_decomposition_prompt("auth and db decisions", "2026-03-07");
        assert!(prompt.contains("auth and db decisions"));
        assert!(prompt.contains("2026-03-07"));
        assert!(prompt.contains("is_multi_faceted"));
        assert!(prompt.contains("sub_queries"));
        assert!(prompt.contains("variants"));
    }
}
