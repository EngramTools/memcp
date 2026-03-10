//! Intelligence layer — embedding, search, recall, and query intelligence.
//!
//! Provides embedding generation (local fastembed + OpenAI), hybrid search
//! with salience scoring, memory recall for context injection, and query
//! expansion/reranking via LLM providers. Feeds from storage/ types,
//! feeds into transport/ (MCP server, CLI).

pub mod embedding;
pub mod query_intelligence;
pub mod recall;
pub mod search;
