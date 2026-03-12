//! Configuration structs loaded from memcp.toml, environment variables, and defaults.
//!
//! Uses figment for layered config (defaults -> file -> env). Every subsystem has its own
//! config struct (EmbeddingConfig, SalienceConfig, GcConfig, etc.) nested under the root Config.

use crate::errors::MemcpError;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for the search subsystem.
///
/// BM25 backend selection is explicit — having ParadeDB installed does NOT auto-switch.
/// Nested env var overrides use double underscores:
///   MEMCP_SEARCH__BM25_BACKEND=paradedb
///   MEMCP_SEARCH__DEFAULT_MIN_SALIENCE=0.5
///   MEMCP_SEARCH__SALIENCE_HINT_MODE=true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// BM25 backend: "native" (PostgreSQL tsvector, default) or "paradedb" (pg_search extension)
    /// Default: "native" — no extension required for self-hosted deployments
    #[serde(default = "default_bm25_backend")]
    pub bm25_backend: String,

    /// Global default minimum salience score for search results (0.0–1.0).
    /// Applied when the caller does not specify min_salience in the request.
    /// When omitted here too, no filtering is applied (backwards compatible).
    #[serde(default)]
    pub default_min_salience: Option<f64>,

    /// When true, empty results that were filtered by salience include a `salience_hint` field
    /// explaining how many results were below the threshold.
    /// Default: false — no hint on empty results.
    #[serde(default)]
    pub salience_hint_mode: bool,
}

fn default_bm25_backend() -> String {
    "native".to_string()
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            bm25_backend: default_bm25_backend(),
            default_min_salience: None,
            salience_hint_mode: false,
        }
    }
}

/// Configuration for the salience scoring subsystem.
///
/// Weights control how much each dimension contributes to the final salience score.
/// All four weights should ideally sum to 1.0 (they are not automatically normalized).
/// Nested env var overrides use double underscores:
///   MEMCP_SALIENCE__W_RECENCY=0.30
///   MEMCP_SALIENCE__DEBUG_SCORING=true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalienceConfig {
    /// Weight for recency dimension (default: 0.25)
    #[serde(default = "default_w_recency")]
    pub w_recency: f64,
    /// Weight for access frequency dimension (default: 0.15)
    #[serde(default = "default_w_access")]
    pub w_access: f64,
    /// Weight for semantic relevance dimension (default: 0.45)
    #[serde(default = "default_w_semantic")]
    pub w_semantic: f64,
    /// Weight for reinforcement strength dimension (default: 0.15)
    #[serde(default = "default_w_reinforce")]
    pub w_reinforce: f64,
    /// Exponential recency decay rate (default: 0.01, ~70-day half-life)
    #[serde(default = "default_recency_lambda")]
    pub recency_lambda: f64,
    /// Enable debug scoring output (shows dimension breakdown in results)
    #[serde(default)]
    pub debug_scoring: bool,
}

fn default_w_recency() -> f64 {
    0.25
}
fn default_w_access() -> f64 {
    0.15
}
fn default_w_semantic() -> f64 {
    0.45
}
fn default_w_reinforce() -> f64 {
    0.15
}
fn default_recency_lambda() -> f64 {
    0.01
}

impl Default for SalienceConfig {
    fn default() -> Self {
        SalienceConfig {
            w_recency: default_w_recency(),
            w_access: default_w_access(),
            w_semantic: default_w_semantic(),
            w_reinforce: default_w_reinforce(),
            recency_lambda: default_recency_lambda(),
            debug_scoring: false,
        }
    }
}

/// Configuration for the extraction pipeline subsystem.
///
/// Provider selection is explicit — "ollama" is the default (local, no API key needed).
/// Nested env var overrides use double underscores:
///   MEMCP_EXTRACTION__PROVIDER=openai
///   MEMCP_EXTRACTION__OPENAI_API_KEY=sk-...
///   MEMCP_EXTRACTION__ENABLED=false
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Which provider to use: "ollama" (local, default) or "openai"
    #[serde(default = "default_extraction_provider")]
    pub provider: String,

    /// Ollama server base URL
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    /// Ollama model for extraction
    #[serde(default = "default_ollama_model")]
    pub ollama_model: String,

    /// OpenAI API key — only required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI model for extraction
    #[serde(default = "default_openai_extraction_model")]
    pub openai_model: String,

    /// Whether extraction is enabled (default: true). Set to false to skip extraction entirely.
    #[serde(default = "default_extraction_enabled")]
    pub enabled: bool,

    /// Maximum content characters to send for extraction (truncated beyond this)
    #[serde(default = "default_max_content_chars")]
    pub max_content_chars: usize,
}

fn default_extraction_provider() -> String {
    "ollama".to_string()
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2:3b".to_string()
}

fn default_openai_extraction_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_extraction_enabled() -> bool {
    true
}

fn default_max_content_chars() -> usize {
    1500
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        ExtractionConfig {
            provider: default_extraction_provider(),
            ollama_base_url: default_ollama_base_url(),
            ollama_model: default_ollama_model(),
            openai_api_key: None,
            openai_model: default_openai_extraction_model(),
            enabled: default_extraction_enabled(),
            max_content_chars: default_max_content_chars(),
        }
    }
}

/// Configuration for the memory consolidation subsystem.
///
/// When enabled, new memories trigger a pgvector similarity check after embedding.
/// If any existing memories exceed the threshold, they are auto-merged via LLM synthesis.
/// Nested env var overrides use double underscores:
///   MEMCP_CONSOLIDATION__ENABLED=false
///   MEMCP_CONSOLIDATION__SIMILARITY_THRESHOLD=0.92
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Whether consolidation is enabled (default: true).
    /// Set to false to disable automatic merging.
    #[serde(default = "default_consolidation_enabled")]
    pub enabled: bool,

    /// Cosine similarity threshold above which memories are merged (default: 0.92).
    /// Range: 0.0–1.0. Higher values require tighter similarity before merging.
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,

    /// Maximum number of originals merged into a single consolidated memory (default: 5).
    #[serde(default = "default_max_consolidation_group")]
    pub max_consolidation_group: usize,
}

fn default_consolidation_enabled() -> bool {
    true
}
fn default_similarity_threshold() -> f64 {
    0.92
}
fn default_max_consolidation_group() -> usize {
    5
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        ConsolidationConfig {
            enabled: default_consolidation_enabled(),
            similarity_threshold: default_similarity_threshold(),
            max_consolidation_group: default_max_consolidation_group(),
        }
    }
}

/// Configuration for the query intelligence subsystem.
///
/// Both expansion and re-ranking are disabled by default — opt in explicitly.
/// Nested env var overrides use double underscores:
///   MEMCP_QUERY_INTELLIGENCE__EXPANSION_ENABLED=true
///   MEMCP_QUERY_INTELLIGENCE__RERANKING_PROVIDER=openai
///   MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY=sk-...
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryIntelligenceConfig {
    /// Enable query expansion (default: false — off by default)
    #[serde(default)]
    pub expansion_enabled: bool,

    /// Enable LLM re-ranking (default: false — off by default)
    #[serde(default)]
    pub reranking_enabled: bool,

    /// Provider for expansion: "ollama" or "openai" (default: "ollama")
    #[serde(default = "default_qi_provider")]
    pub expansion_provider: String,

    /// Provider for reranking: "ollama" or "openai" (default: "ollama")
    #[serde(default = "default_qi_provider")]
    pub reranking_provider: String,

    /// Ollama base URL (shared with extraction config but independently overridable)
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    /// Ollama model for expansion
    #[serde(default = "default_qi_ollama_model")]
    pub expansion_ollama_model: String,

    /// Ollama model for reranking
    #[serde(default = "default_qi_ollama_model")]
    pub reranking_ollama_model: String,

    /// OpenAI-compatible base URL (supports Kimi, custom endpoints)
    #[serde(default = "default_qi_openai_base_url")]
    pub openai_base_url: String,

    /// OpenAI-compatible API key — only required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI model for expansion
    #[serde(default = "default_qi_openai_model")]
    pub expansion_openai_model: String,

    /// OpenAI model for reranking
    #[serde(default = "default_qi_openai_model")]
    pub reranking_openai_model: String,

    /// Enable multi-query decomposition (default: true).
    /// When enabled, complex queries are decomposed into sub-queries merged via RRF.
    /// Set to false to always use single-query expansion (legacy behavior).
    #[serde(default = "default_true")]
    pub multi_query_enabled: bool,

    /// Maximum combined latency budget in ms (default: 2000)
    #[serde(default = "default_latency_budget_ms")]
    pub latency_budget_ms: u64,

    /// Max content chars sent to re-ranker per candidate (default: 500)
    #[serde(default = "default_rerank_content_chars")]
    pub rerank_content_chars: usize,
}

fn default_true() -> bool {
    true
}

fn default_qi_provider() -> String {
    "ollama".to_string()
}

fn default_qi_ollama_model() -> String {
    "llama3.2:3b".to_string()
}

fn default_qi_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_qi_openai_model() -> String {
    "gpt-5-mini".to_string()
}

fn default_latency_budget_ms() -> u64 {
    2000
}

fn default_rerank_content_chars() -> usize {
    500
}

impl Default for QueryIntelligenceConfig {
    fn default() -> Self {
        QueryIntelligenceConfig {
            expansion_enabled: false,
            reranking_enabled: false,
            expansion_provider: default_qi_provider(),
            reranking_provider: default_qi_provider(),
            ollama_base_url: default_ollama_base_url(),
            expansion_ollama_model: default_qi_ollama_model(),
            reranking_ollama_model: default_qi_ollama_model(),
            openai_base_url: default_qi_openai_base_url(),
            openai_api_key: None,
            expansion_openai_model: default_qi_openai_model(),
            reranking_openai_model: default_qi_openai_model(),
            multi_query_enabled: default_true(),
            latency_budget_ms: default_latency_budget_ms(),
            rerank_content_chars: default_rerank_content_chars(),
        }
    }
}

/// Routing rules that determine when an embedding tier is selected at store time.
///
/// All specified conditions must be met (AND logic). Omitted conditions are not checked.
/// Example: `{ min_stability = 0.8, type_hints = ["decision"] }` matches memories
/// with stability >= 0.8 AND type_hint = "decision".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingConfig {
    /// Minimum stability score for this tier (None = no minimum)
    #[serde(default)]
    pub min_stability: Option<f64>,
    /// Memory type_hints that should use this tier
    #[serde(default)]
    pub type_hints: Vec<String>,
    /// Minimum content length (chars) for this tier
    #[serde(default)]
    pub min_content_length: Option<usize>,
}

/// Configuration for the promotion sweep that upgrades memories from a lower tier
/// to a higher-quality tier based on importance signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionConfig {
    /// Minimum reinforcement count to promote
    #[serde(default = "default_min_reinforcements")]
    pub min_reinforcements: u32,
    /// Minimum stability score to promote
    #[serde(default = "default_min_stability_promotion")]
    pub min_stability: f64,
    /// Sweep interval in minutes
    #[serde(default = "default_sweep_interval_minutes")]
    pub sweep_interval_minutes: u64,
    /// Max promotions per sweep cycle
    #[serde(default = "default_batch_cap")]
    pub batch_cap: usize,
}

fn default_min_reinforcements() -> u32 {
    3
}
fn default_min_stability_promotion() -> f64 {
    0.8
}
fn default_sweep_interval_minutes() -> u64 {
    60
}
fn default_batch_cap() -> usize {
    15
}

impl Default for PromotionConfig {
    fn default() -> Self {
        PromotionConfig {
            min_reinforcements: default_min_reinforcements(),
            min_stability: default_min_stability_promotion(),
            sweep_interval_minutes: default_sweep_interval_minutes(),
            batch_cap: default_batch_cap(),
        }
    }
}

/// Configuration for a single embedding tier in a multi-model setup.
///
/// Each tier represents a different embedding model (e.g., fast local model vs quality API model).
/// Example TOML:
/// ```toml
/// [embedding.tiers.fast]
/// provider = "local"
/// model = "AllMiniLML6V2"
///
/// [embedding.tiers.quality]
/// provider = "openai"
/// model = "text-embedding-3-small"
/// routing = { type_hints = ["decision"], min_stability = 0.8 }
/// promotion = { min_reinforcements = 3, min_stability = 0.8, sweep_interval_minutes = 60, batch_cap = 15 }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingTierConfig {
    /// Provider type: "local" or "openai"
    pub provider: String,
    /// Model name (fastembed identifier or OpenAI model name)
    #[serde(default)]
    pub model: Option<String>,
    /// OpenAI API key override (falls back to top-level if not set)
    #[serde(default)]
    pub openai_api_key: Option<String>,
    /// API base URL override (defaults to OpenAI; use Google's endpoint for Gemini embeddings)
    #[serde(default)]
    pub base_url: Option<String>,
    /// Vector dimension override (auto-detected from model if omitted)
    #[serde(default)]
    pub dimension: Option<usize>,
    /// Routing rules (when this tier is used at store time)
    #[serde(default)]
    pub routing: Option<RoutingConfig>,
    /// Promotion rules (for sweep worker to promote from lower tier)
    #[serde(default)]
    pub promotion: Option<PromotionConfig>,
}

/// Configuration for the embedding provider subsystem.
///
/// Provider selection is explicit — having an API key does NOT auto-switch from local.
/// Nested env var overrides use double underscores:
///   MEMCP_EMBEDDING__PROVIDER=openai
///   MEMCP_EMBEDDING__OPENAI_API_KEY=sk-...
///   MEMCP_EMBEDDING__LOCAL_MODEL=BGEBaseENV15
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Which provider to use: "local" (fastembed) or "openai"
    /// Default: "local" — no API key required for self-hosted deployments
    #[serde(default = "default_embedding_provider")]
    pub provider: String,

    /// OpenAI API key — only required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// Directory for caching model weights (fastembed downloads)
    /// Default: platform cache dir + "/memcp/models", fallback to /tmp/memcp_models
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    /// Local embedding model name (fastembed identifier).
    /// Default: "AllMiniLML6V2" (384 dimensions, all-MiniLM-L6-v2)
    #[serde(default = "default_local_model")]
    pub local_model: String,

    /// OpenAI embedding model name.
    /// Default: "text-embedding-3-small" (1536 dimensions)
    #[serde(default = "default_openai_model")]
    pub openai_model: String,

    /// API base URL for OpenAI-compatible embedding providers.
    /// Default: "https://api.openai.com/v1"
    /// For Google Gemini: "https://generativelanguage.googleapis.com/v1beta/openai"
    #[serde(default)]
    pub openai_base_url: Option<String>,

    /// Override vector dimension. Auto-detected from model if omitted.
    /// Only needed for custom/unknown models.
    #[serde(default)]
    pub dimension: Option<usize>,

    /// Re-embed when only tags change (default: false).
    /// When false, only content changes trigger re-embedding. Tag-only updates
    /// skip re-embed to save compute (symbolic search still works for tags).
    #[serde(default)]
    pub reembed_on_tag_change: bool,

    /// Named embedding tiers for multi-model support.
    /// Empty = legacy single-model mode (uses provider/local_model/openai_model above).
    /// Example: `[embedding.tiers.fast]`, `[embedding.tiers.quality]`
    #[serde(default)]
    pub tiers: HashMap<String, EmbeddingTierConfig>,
}

fn default_embedding_provider() -> String {
    "local".to_string()
}

fn default_cache_dir() -> String {
    dirs::cache_dir()
        .map(|p| {
            p.join("memcp")
                .join("models")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "/tmp/memcp_models".to_string())
}

fn default_local_model() -> String {
    "AllMiniLML6V2".to_string()
}

fn default_openai_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            provider: default_embedding_provider(),
            openai_api_key: None,
            cache_dir: default_cache_dir(),
            local_model: default_local_model(),
            openai_model: default_openai_model(),
            openai_base_url: None,
            dimension: None,
            reembed_on_tag_change: false,
            tiers: HashMap::new(),
        }
    }
}

impl EmbeddingConfig {
    /// Returns true if multiple embedding tiers are configured (multi-model mode).
    pub fn is_multi_model(&self) -> bool {
        self.tiers.len() > 1
    }

    /// Returns the default tier name.
    /// "fast" if tiers is non-empty, otherwise the top-level provider name.
    pub fn default_tier_name(&self) -> &str {
        if !self.tiers.is_empty() {
            if self.tiers.contains_key("fast") {
                "fast"
            } else {
                self.tiers
                    .keys()
                    .next()
                    .map(|s| s.as_str())
                    .unwrap_or("fast")
            }
        } else {
            &self.provider
        }
    }
}

/// Configuration for the category-aware auto-store filter.
///
/// Controls the heuristic-based category filter that blocks tool narration
/// (e.g. "Let me read the file...", "Now I'll edit...") while passing through
/// valuable content (decisions, preferences, errors, architecture notes).
/// Enabled by default when filter_mode = "category".
/// Nested env var overrides use double underscores:
///   MEMCP_AUTO_STORE__CATEGORY_FILTER__ENABLED=false
///   MEMCP_AUTO_STORE__CATEGORY_FILTER__BLOCK_TOOL_NARRATION=false
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryFilterConfig {
    /// Whether category filtering is enabled (default: true)
    #[serde(default = "default_category_filter_enabled")]
    pub enabled: bool,

    /// Block tool narration patterns (default: true).
    /// When true, phrases like "Let me read...", "Now I'll edit..." are filtered out.
    #[serde(default = "default_block_tool_narration")]
    pub block_tool_narration: bool,

    /// Additional custom regex patterns to block, beyond the built-in defaults.
    /// Each pattern is applied to the start of entry content (anchored at ^).
    /// Invalid patterns are skipped with a warning (fail-open).
    #[serde(default)]
    pub tool_narration_patterns: Vec<String>,

    /// Per-category actions: "store", "skip", or "store-low" (store with lower stability).
    /// Keys are category names from the taxonomy (decision, preference, architecture,
    /// fact, instruction, correction, tool-narration, ephemeral, code-output, error-trace).
    #[serde(default = "default_category_actions")]
    pub category_actions: std::collections::HashMap<String, String>,

    /// LLM provider for category classification: "ollama" or "openai".
    /// Uses extraction config for base URLs and API keys.
    /// When None, LLM classification is disabled (heuristic-only).
    #[serde(default)]
    pub llm_provider: Option<String>,

    /// LLM model for category classification (e.g. "llama3.2", "gpt-4o-mini").
    #[serde(default)]
    pub llm_model: Option<String>,
}

fn default_category_filter_enabled() -> bool {
    true
}
fn default_block_tool_narration() -> bool {
    true
}

fn default_category_actions() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("decision".to_string(), "store".to_string());
    m.insert("preference".to_string(), "store".to_string());
    m.insert("architecture".to_string(), "store".to_string());
    m.insert("fact".to_string(), "store".to_string());
    m.insert("instruction".to_string(), "store".to_string());
    m.insert("correction".to_string(), "store".to_string());
    m.insert("tool-narration".to_string(), "skip".to_string());
    m.insert("ephemeral".to_string(), "skip".to_string());
    m.insert("code-output".to_string(), "store-low".to_string());
    m.insert("error-trace".to_string(), "store-low".to_string());
    m
}

impl Default for CategoryFilterConfig {
    fn default() -> Self {
        CategoryFilterConfig {
            enabled: default_category_filter_enabled(),
            block_tool_narration: default_block_tool_narration(),
            tool_narration_patterns: Vec::new(),
            category_actions: default_category_actions(),
            llm_provider: None,
            llm_model: None,
        }
    }
}

/// Configuration for the auto-store sidecar.
///
/// Watches conversation log files and automatically ingests memories
/// without requiring the agent to explicitly call store_memory.
/// Disabled by default — opt in via `[auto_store] enabled = true`.
/// Nested env var overrides use double underscores:
///   MEMCP_AUTO_STORE__ENABLED=true
///   MEMCP_AUTO_STORE__WATCH_PATHS=~/.claude/history.jsonl
///   MEMCP_AUTO_STORE__FILTER_MODE=heuristic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStoreConfig {
    /// Whether auto-store is enabled (default: false — opt-in)
    #[serde(default)]
    pub enabled: bool,

    /// Paths to watch for new log entries.
    /// Supports ~ expansion for home directory.
    /// e.g. ["~/.claude/history.jsonl"]
    #[serde(default)]
    pub watch_paths: Vec<String>,

    /// Log format: "claude-code" (default) or "generic-jsonl"
    #[serde(default = "default_auto_store_format")]
    pub format: String,

    /// Filter mode: "llm" (default), "heuristic", "category", or "none"
    #[serde(default = "default_auto_store_filter_mode")]
    pub filter_mode: String,

    /// LLM provider for filtering: "ollama" (default) or "openai"
    #[serde(default = "default_auto_store_filter_provider")]
    pub filter_provider: String,

    /// Model name for LLM filter (e.g. "llama3.2")
    #[serde(default = "default_auto_store_filter_model")]
    pub filter_model: String,

    /// Fallback poll interval in seconds (default: 5)
    #[serde(default = "default_auto_store_poll_interval")]
    pub poll_interval_secs: u64,

    /// Dedup window in seconds — identical content within this window is skipped (default: 300)
    #[serde(default = "default_auto_store_dedup_window")]
    pub dedup_window_secs: u64,

    /// Category filter configuration (used when filter_mode = "category").
    #[serde(default)]
    pub category_filter: CategoryFilterConfig,
}

fn default_auto_store_format() -> String {
    "claude-code".to_string()
}
fn default_auto_store_filter_mode() -> String {
    "none".to_string()
}
fn default_auto_store_filter_provider() -> String {
    "ollama".to_string()
}
fn default_auto_store_filter_model() -> String {
    "llama3.2".to_string()
}
fn default_auto_store_poll_interval() -> u64 {
    5
}
fn default_auto_store_dedup_window() -> u64 {
    300
}

impl Default for AutoStoreConfig {
    fn default() -> Self {
        AutoStoreConfig {
            enabled: false,
            watch_paths: Vec::new(),
            format: default_auto_store_format(),
            filter_mode: default_auto_store_filter_mode(),
            filter_provider: default_auto_store_filter_provider(),
            filter_model: default_auto_store_filter_model(),
            poll_interval_secs: default_auto_store_poll_interval(),
            dedup_window_secs: default_auto_store_dedup_window(),
            category_filter: CategoryFilterConfig::default(),
        }
    }
}

/// Configuration for the semantic deduplication subsystem.
///
/// After embedding completes for a new memory, the dedup worker checks similarity
/// against all existing embedded memories. Near-duplicates (above similarity_threshold)
/// are merged: the existing memory metadata is updated, the new memory is soft-deleted,
/// and all contributing sources are tracked in dedup_sources JSONB.
/// Dedup is async (zero ingest latency impact). Fail-open: errors are logged, never cause data loss.
/// Nested env var overrides use double underscores:
///   MEMCP_DEDUP__ENABLED=false
///   MEMCP_DEDUP__SIMILARITY_THRESHOLD=0.97
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    /// Whether semantic dedup is enabled (default: true — runs automatically after embedding).
    #[serde(default = "default_dedup_enabled")]
    pub enabled: bool,

    /// Cosine similarity threshold above which memories are considered near-duplicates (default: 0.95).
    /// Stricter than consolidation's 0.92 — avoids false positives on paraphrases.
    /// Range: 0.0–1.0. Higher values require tighter similarity before merging.
    #[serde(default = "default_dedup_similarity_threshold")]
    pub similarity_threshold: f64,
}

fn default_dedup_enabled() -> bool {
    true
}
fn default_dedup_similarity_threshold() -> f64 {
    0.95
}

impl Default for DedupConfig {
    fn default() -> Self {
        DedupConfig {
            enabled: default_dedup_enabled(),
            similarity_threshold: default_dedup_similarity_threshold(),
        }
    }
}

/// Configuration for memory chunking.
///
/// When enabled, long auto-store content is split into overlapping sentence-grouped
/// chunks with separate embeddings for better retrieval granularity. Only affects
/// auto-store ingestion — explicit store operations are never chunked.
///
/// Nested env var overrides use double underscores:
///   MEMCP_CHUNKING__ENABLED=false
///   MEMCP_CHUNKING__MAX_CHUNK_CHARS=1024
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Enable chunking for auto-store content (default: true)
    #[serde(default = "default_chunking_enabled")]
    pub enabled: bool,

    /// Maximum characters per chunk (~4 chars/token, so 1024 chars ~= 256 tokens).
    #[serde(default = "default_max_chunk_chars")]
    pub max_chunk_chars: usize,

    /// Number of sentences to overlap between adjacent chunks (default: 2).
    #[serde(default = "default_overlap_sentences")]
    pub overlap_sentences: usize,

    /// Minimum content length (in chars) to trigger chunking.
    /// Content shorter than this is stored as a single memory.
    /// ~512 tokens = ~2048 chars.
    #[serde(default = "default_min_content_chars")]
    pub min_content_chars: usize,
}

fn default_chunking_enabled() -> bool {
    true
}
fn default_max_chunk_chars() -> usize {
    1024
}
fn default_overlap_sentences() -> usize {
    2
}
fn default_min_content_chars() -> usize {
    2048
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        ChunkingConfig {
            enabled: default_chunking_enabled(),
            max_chunk_chars: default_max_chunk_chars(),
            overlap_sentences: default_overlap_sentences(),
            min_content_chars: default_min_content_chars(),
        }
    }
}

/// Configuration for the recall subsystem (automatic context injection).
///
/// Recall surfaces the most relevant memories at the start of a session
/// and applies a lightweight salience bump to recalled memories.
/// Disabled fields are always valid — configs without [recall] use defaults.
/// Nested env var overrides use double underscores:
///   MEMCP_RECALL__MAX_MEMORIES=5
///   MEMCP_RECALL__MIN_RELEVANCE=0.6
///   MEMCP_RECALL__BUMP_MULTIPLIER=0.20
///   MEMCP_RECALL__TRUNCATION_CHARS=200
///   MEMCP_RECALL__RELATED_CONTEXT_ENABLED=false
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallConfig {
    /// Maximum number of memories to return per recall (default: 3)
    #[serde(default = "default_recall_max_memories")]
    pub max_memories: usize,
    /// Minimum relevance threshold for recall results, 0.0–1.0 (default: 0.7)
    #[serde(default = "default_recall_min_relevance")]
    pub min_relevance: f64,
    /// Session idle expiry in seconds (default: 86400 — 24 hours).
    /// Sessions inactive longer than this are cleaned up by the GC worker.
    #[serde(default = "default_recall_session_idle_secs")]
    pub session_idle_secs: u64,
    /// Stability multiplier for recall salience bump (default: 0.15).
    /// On recall, stability = stability * (1.0 + bump_multiplier).
    /// Smaller than reinforce (1.5×) — passive implicit signal.
    #[serde(default = "default_recall_bump_multiplier")]
    pub bump_multiplier: f64,
    /// Maximum stability value — recall bump stops here (default: 100.0).
    /// Prevents unbounded growth for frequently-recalled memories.
    #[serde(default = "default_recall_stability_ceiling")]
    pub stability_ceiling: f64,
    /// Maximum characters to return per memory content in recall output (default: 200).
    /// Content longer than this is truncated with "..." indicator.
    /// Agent can use `memcp get <id>` for full content. Saves tokens on large memories.
    #[serde(default = "default_recall_truncation_chars")]
    pub truncation_chars: usize,
    /// Custom preamble text for `memcp recall --first` output.
    /// When None, a hardcoded sensible default is used.
    /// Allows operators to customize the session-start context injected into agent prompts.
    #[serde(default)]
    pub preamble_override: Option<String>,
    /// Whether to compute related_count and hint per recalled memory (default: true).
    /// When enabled, each recalled memory includes a count of memories sharing at least
    /// one non-trivial tag and a ready-made search command for the agent to explore.
    #[serde(default = "default_recall_related_context_enabled")]
    pub related_context_enabled: bool,
    /// Weight per matching boost tag (default 0.1). Additive per match.
    /// Memories sharing N boost tags get N * tag_boost_weight added to their score.
    #[serde(default = "default_tag_boost_weight")]
    pub tag_boost_weight: f64,
    /// Weight per matching session-accumulated tag (default 0.05). Lighter than explicit.
    /// Session tags are accumulated from recalled memories within the session.
    #[serde(default = "default_session_boost_weight")]
    pub session_boost_weight: f64,
    /// Maximum total explicit tag boost (default 0.3). Prevents override of strong salience signals.
    #[serde(default = "default_tag_boost_cap")]
    pub tag_boost_cap: f64,
    /// Maximum total session tag boost (default 0.15).
    #[serde(default = "default_session_boost_cap")]
    pub session_boost_cap: f64,
    /// Enable session topic accumulation (default true). When true, recalled memory tags
    /// are cached on the session for implicit boosting on subsequent recalls.
    #[serde(default = "default_session_topic_tracking")]
    pub session_topic_tracking: bool,
}

fn default_recall_max_memories() -> usize {
    3
}
fn default_recall_min_relevance() -> f64 {
    0.7
}
fn default_recall_session_idle_secs() -> u64 {
    86400
}
fn default_recall_bump_multiplier() -> f64 {
    0.15
}
fn default_recall_stability_ceiling() -> f64 {
    100.0
}
fn default_recall_truncation_chars() -> usize {
    200
}
fn default_recall_related_context_enabled() -> bool {
    true
}
fn default_tag_boost_weight() -> f64 {
    0.1
}
fn default_session_boost_weight() -> f64 {
    0.05
}
fn default_tag_boost_cap() -> f64 {
    0.3
}
fn default_session_boost_cap() -> f64 {
    0.15
}
fn default_session_topic_tracking() -> bool {
    true
}

impl Default for RecallConfig {
    fn default() -> Self {
        RecallConfig {
            max_memories: default_recall_max_memories(),
            min_relevance: default_recall_min_relevance(),
            session_idle_secs: default_recall_session_idle_secs(),
            bump_multiplier: default_recall_bump_multiplier(),
            stability_ceiling: default_recall_stability_ceiling(),
            truncation_chars: default_recall_truncation_chars(),
            preamble_override: None,
            related_context_enabled: default_recall_related_context_enabled(),
            tag_boost_weight: default_tag_boost_weight(),
            session_boost_weight: default_session_boost_weight(),
            tag_boost_cap: default_tag_boost_cap(),
            session_boost_cap: default_session_boost_cap(),
            session_topic_tracking: default_session_topic_tracking(),
        }
    }
}

/// Configuration for the idempotency subsystem.
///
/// Controls content-hash dedup window for store operations (IDP-01) and
/// caller-provided idempotency key TTL and length limits (IDP-02).
/// Nested env var overrides use double underscores:
///   MEMCP_IDEMPOTENCY__DEDUP_WINDOW_SECS=120
///   MEMCP_IDEMPOTENCY__KEY_TTL_SECS=3600
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyConfig {
    /// Time window in seconds during which identical content (same hash) is deduplicated.
    /// Default: 60 seconds. Set to 0 to disable content-hash dedup.
    #[serde(default = "default_dedup_window_secs")]
    pub dedup_window_secs: u64,

    /// TTL for idempotency keys in seconds (default: 86400 — 24 hours).
    /// Keys older than this are eligible for cleanup by the GC worker.
    #[serde(default = "default_key_ttl_secs")]
    pub key_ttl_secs: u64,

    /// Maximum allowed length for a caller-provided idempotency_key (default: 256 bytes).
    /// Requests with longer keys return a Validation error.
    #[serde(default = "default_max_key_length")]
    pub max_key_length: usize,
}

fn default_dedup_window_secs() -> u64 {
    60
}
fn default_key_ttl_secs() -> u64 {
    86400
}
fn default_max_key_length() -> usize {
    256
}

impl Default for IdempotencyConfig {
    fn default() -> Self {
        IdempotencyConfig {
            dedup_window_secs: default_dedup_window_secs(),
            key_ttl_secs: default_key_ttl_secs(),
            max_key_length: default_max_key_length(),
        }
    }
}

/// Configuration for user-specific context that improves memory resolution.
///
/// Provides personal context for temporal extraction and other personalization features.
/// Nested env var overrides use double underscores:
///   MEMCP_USER__BIRTH_YEAR=1990
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserConfig {
    /// User's birth year, used to resolve relative-age references in memories.
    /// Example: "when I was 6" + birth_year=1990 resolves to event_time=1996.
    /// When None, relative-age references are stored without temporal resolution.
    #[serde(default)]
    pub birth_year: Option<u32>,
}

/// Configuration for project scoping.
///
/// Projects isolate memories by codebase or context. NULL project = global (always visible).
/// Activation precedence: CLI flag (--project) > env var (MEMCP_PROJECT) > this config default.
/// Nested env var overrides use double underscores:
///   MEMCP_PROJECT__DEFAULT_PROJECT=myproject
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Default project applied when no CLI flag or env var is set.
    /// NULL (None) means global — all memories are stored without project scoping.
    #[serde(default)]
    pub default_project: Option<String>,
}

/// Configuration for temporal event time extraction.
///
/// Controls whether a background LLM worker extracts event_time from memory content
/// for subtler references ("back in college", "during COVID") that regex misses.
/// Regex-based extraction (fast, deterministic) always runs inline during store.
/// Nested env var overrides use double underscores:
///   MEMCP_TEMPORAL__LLM_ENABLED=true
///   MEMCP_TEMPORAL__PROVIDER=ollama
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalConfig {
    /// Whether the LLM background worker for subtle temporal extraction is enabled (default: false).
    /// When false, only regex-based extraction runs (catches "in 2019", "when I was 6", etc.).
    /// When true, a daemon worker does a second pass for subtler references.
    #[serde(default)]
    pub llm_enabled: bool,

    /// Provider for LLM temporal extraction: "ollama" (default) or "openai"
    #[serde(default = "default_temporal_provider")]
    pub provider: String,

    /// Ollama model for temporal extraction
    #[serde(default = "default_temporal_ollama_model")]
    pub ollama_model: String,

    /// OpenAI model for temporal extraction
    #[serde(default = "default_temporal_openai_model")]
    pub openai_model: String,

    /// OpenAI-compatible API key — only required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI-compatible base URL (supports custom endpoints)
    #[serde(default = "default_temporal_openai_base_url")]
    pub openai_base_url: Option<String>,
}

fn default_temporal_provider() -> String {
    "ollama".to_string()
}
fn default_temporal_ollama_model() -> String {
    "llama3.2:3b".to_string()
}
fn default_temporal_openai_model() -> String {
    "gpt-4o-mini".to_string()
}
fn default_temporal_openai_base_url() -> Option<String> {
    None
}

impl Default for TemporalConfig {
    fn default() -> Self {
        TemporalConfig {
            llm_enabled: false,
            provider: default_temporal_provider(),
            ollama_model: default_temporal_ollama_model(),
            openai_model: default_temporal_openai_model(),
            openai_api_key: None,
            openai_base_url: default_temporal_openai_base_url(),
        }
    }
}

/// Configuration for the garbage collection subsystem.
///
/// GC runs on a schedule (gc_interval_secs) and prunes memories that are
/// both low-salience (below salience_threshold) AND older than min_age_days.
/// Never prunes below min_memory_floor to protect small knowledge bases.
/// Soft-deleted memories are hard-purged after hard_purge_grace_days.
/// Nested env var overrides use double underscores:
///   MEMCP_GC__ENABLED=false
///   MEMCP_GC__SALIENCE_THRESHOLD=0.5
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// Whether GC is enabled (default: true — runs automatically)
    #[serde(default = "default_gc_enabled")]
    pub enabled: bool,

    /// Absolute FSRS stability threshold below which memories are candidates for pruning.
    /// Default: 0.3 — memories with stability < 0.3 are low-salience.
    #[serde(default = "default_gc_salience_threshold")]
    pub salience_threshold: f64,

    /// Minimum memory age in days before a memory can be pruned (default: 30).
    /// Fresh memories are never pruned even if salience is low.
    #[serde(default = "default_gc_min_age_days")]
    pub min_age_days: u32,

    /// Minimum number of live memories to retain (default: 100).
    /// GC never prunes below this count — small knowledge bases are never touched.
    #[serde(default = "default_gc_min_memory_floor")]
    pub min_memory_floor: u64,

    /// How often to run GC in seconds (default: 3600 — once per hour).
    #[serde(default = "default_gc_interval_secs")]
    pub gc_interval_secs: u64,

    /// Days after soft-delete before hard purge (default: 30).
    /// Soft-deleted memories are excluded from all queries but remain recoverable
    /// until the grace period expires.
    #[serde(default = "default_gc_hard_purge_grace_days")]
    pub hard_purge_grace_days: u32,
}

fn default_gc_enabled() -> bool {
    true
}
fn default_gc_salience_threshold() -> f64 {
    0.3
}
fn default_gc_min_age_days() -> u32 {
    30
}
fn default_gc_min_memory_floor() -> u64 {
    100
}
fn default_gc_interval_secs() -> u64 {
    3600
}
fn default_gc_hard_purge_grace_days() -> u32 {
    30
}

impl Default for GcConfig {
    fn default() -> Self {
        GcConfig {
            enabled: default_gc_enabled(),
            salience_threshold: default_gc_salience_threshold(),
            min_age_days: default_gc_min_age_days(),
            min_memory_floor: default_gc_min_memory_floor(),
            gc_interval_secs: default_gc_interval_secs(),
            hard_purge_grace_days: default_gc_hard_purge_grace_days(),
        }
    }
}

/// Configuration for content filtering (topic exclusion).
///
/// Two-tier system: regex patterns (fast, deterministic) and
/// semantic topic exclusion (embedding-based similarity).
/// Disabled by default — opt in via `[content_filter] enabled = true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentFilterConfig {
    /// Whether content filtering is enabled (default: false — opt-in)
    #[serde(default)]
    pub enabled: bool,

    /// Default action when content matches: "drop" (silent, default) or "reject" (return error)
    #[serde(default = "default_filter_action")]
    pub default_action: String,

    /// Regex patterns — content matching ANY pattern is excluded.
    #[serde(default)]
    pub regex_patterns: Vec<String>,

    /// Semantic topics to exclude. Each string is embedded at startup
    /// and incoming content is checked via cosine similarity.
    #[serde(default)]
    pub excluded_topics: Vec<String>,

    /// Cosine similarity threshold for semantic exclusion (default: 0.85).
    #[serde(default = "default_exclusion_threshold")]
    pub semantic_threshold: f64,
}

fn default_filter_action() -> String {
    "drop".to_string()
}

fn default_exclusion_threshold() -> f64 {
    0.85
}

impl Default for ContentFilterConfig {
    fn default() -> Self {
        ContentFilterConfig {
            enabled: false,
            default_action: default_filter_action(),
            regex_patterns: Vec::new(),
            excluded_topics: Vec::new(),
            semantic_threshold: default_exclusion_threshold(),
        }
    }
}

/// Configuration for the Claude Code status line integration.
///
/// Controls the format of the status line output script.
/// Nested env var overrides use double underscores:
///   MEMCP_STATUS_LINE__FORMAT=pending
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusLineConfig {
    /// Format: "ingest" (default), "pending", or "state"
    #[serde(default = "default_statusline_format")]
    pub format: String,
}

fn default_statusline_format() -> String {
    "ingest".to_string()
}

impl Default for StatusLineConfig {
    fn default() -> Self {
        StatusLineConfig {
            format: default_statusline_format(),
        }
    }
}

/// Configuration for the auto-summarization subsystem.
///
/// When enabled, the auto-store sidecar summarizes AI assistant responses
/// before storing them as memories. Uses Ollama (local, default) or any
/// OpenAI-compatible API (OpenAI, Kimi/Moonshot, local vLLM, etc.).
/// Disabled by default — opt in via `[summarization] enabled = true`.
/// Nested env var overrides use double underscores:
///   MEMCP_SUMMARIZATION__ENABLED=true
///   MEMCP_SUMMARIZATION__PROVIDER=openai
///   MEMCP_SUMMARIZATION__OPENAI_BASE_URL=https://api.moonshot.cn/v1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationConfig {
    /// Whether summarization is enabled (default: false — opt-in)
    #[serde(default)]
    pub enabled: bool,

    /// Which provider to use: "ollama" (local, default) or "openai" (any OpenAI-compatible API)
    #[serde(default = "default_summarization_provider")]
    pub provider: String,

    /// Ollama server base URL
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    /// Ollama model for summarization
    #[serde(default = "default_summarization_ollama_model")]
    pub ollama_model: String,

    /// OpenAI-compatible base URL (supports OpenAI, Kimi/Moonshot, local vLLM, etc.)
    #[serde(default = "default_summarization_openai_base_url")]
    pub openai_base_url: String,

    /// OpenAI-compatible API key — required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI-compatible model for summarization (e.g. "gpt-4o-mini", "kimi-k2.5")
    #[serde(default = "default_summarization_openai_model")]
    pub openai_model: String,

    /// Maximum input characters before truncation (default: 4000)
    #[serde(default = "default_summarization_max_input_chars")]
    pub max_input_chars: usize,

    /// System prompt template for summarization.
    /// The assistant response content is appended after this prompt.
    #[serde(default = "default_summarization_prompt")]
    pub prompt_template: String,
}

fn default_summarization_provider() -> String {
    "ollama".to_string()
}
fn default_summarization_ollama_model() -> String {
    "llama3.2:3b".to_string()
}
fn default_summarization_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_summarization_openai_model() -> String {
    "gpt-4o-mini".to_string()
}
fn default_summarization_max_input_chars() -> usize {
    4000
}
fn default_summarization_prompt() -> String {
    "Summarize the following AI assistant response into a concise memory entry. \
     Extract and preserve:\n\
     - Decisions made and their reasoning\n\
     - Technical facts, explanations, and architecture choices\n\
     - User preferences and corrections\n\
     - Action items and next steps\n\
     - Commands, configurations, or patterns worth remembering\n\n\
     Omit:\n\
     - Verbose reasoning chains and thinking-out-loud\n\
     - Code formatting noise (keep only key snippets if critical)\n\
     - Repeated context the user already knows\n\
     - Pleasantries and filler\n\n\
     Output a concise paragraph (2-5 sentences). If the response contains multiple \
     distinct topics, separate them with semicolons."
        .to_string()
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        SummarizationConfig {
            enabled: false,
            provider: default_summarization_provider(),
            ollama_base_url: default_ollama_base_url(),
            ollama_model: default_summarization_ollama_model(),
            openai_base_url: default_summarization_openai_base_url(),
            openai_api_key: None,
            openai_model: default_summarization_openai_model(),
            max_input_chars: default_summarization_max_input_chars(),
            prompt_template: default_summarization_prompt(),
        }
    }
}

/// Configuration for the tiered content abstraction subsystem.
///
/// Generates L0 abstracts (~100 tokens) and optional L1 overviews (~500 tokens)
/// for memory entries, improving semantic search quality and enabling tiered context loading.
/// Disabled by default — opt in via `[abstraction] enabled = true`.
/// Nested env var overrides use double underscores:
///   MEMCP_ABSTRACTION__ENABLED=true
///   MEMCP_ABSTRACTION__PROVIDER=openai
///   MEMCP_ABSTRACTION__GENERATE_OVERVIEW=true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractionConfig {
    /// Whether abstraction is enabled (default: false — opt-in)
    #[serde(default)]
    pub enabled: bool,

    /// Which provider to use: "ollama" (local, default) or "openai" (any OpenAI-compatible API)
    #[serde(default = "default_abstraction_provider")]
    pub provider: String,

    /// Whether to generate L1 overviews in addition to L0 abstracts (default: false)
    /// When false, only abstract_text (L0) is generated.
    #[serde(default)]
    pub generate_overview: bool,

    /// System prompt template for L0 abstract generation.
    /// {content} is replaced with the (truncated) memory content.
    #[serde(default = "default_abstract_prompt_template")]
    pub abstract_prompt_template: String,

    /// System prompt template for L1 overview generation.
    /// {content} is replaced with the (truncated) memory content.
    #[serde(default = "default_overview_prompt_template")]
    pub overview_prompt_template: String,

    /// Ollama server base URL
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    /// Ollama model for abstraction
    #[serde(default = "default_abstraction_ollama_model")]
    pub ollama_model: String,

    /// OpenAI-compatible base URL (supports OpenAI, Kimi/Moonshot, local vLLM, etc.)
    #[serde(default = "default_abstraction_openai_base_url")]
    pub openai_base_url: String,

    /// OpenAI-compatible API key — required when provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI-compatible model for abstraction (e.g. "gpt-4o-mini")
    #[serde(default = "default_abstraction_openai_model")]
    pub openai_model: String,

    /// Maximum input characters before truncation (default: 4000)
    #[serde(default = "default_abstraction_max_input_chars")]
    pub max_input_chars: usize,

    /// Minimum content length (chars) to trigger abstraction (default: 200).
    /// Memories shorter than this are marked 'skipped' — abstraction adds no value.
    #[serde(default = "default_abstraction_min_content_length")]
    pub min_content_length: usize,
}

fn default_abstraction_provider() -> String {
    "ollama".to_string()
}
fn default_abstraction_ollama_model() -> String {
    "llama3.2:3b".to_string()
}
fn default_abstraction_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_abstraction_openai_model() -> String {
    "gpt-4o-mini".to_string()
}
fn default_abstraction_max_input_chars() -> usize {
    4000
}
fn default_abstraction_min_content_length() -> usize {
    200
}
fn default_abstract_prompt_template() -> String {
    "Summarize the following memory into a single concise sentence (under 100 tokens) \
     that captures the key information for semantic search: {content}"
        .to_string()
}
fn default_overview_prompt_template() -> String {
    "Create a structured overview of the following memory in 3-5 bullet points \
     (under 500 tokens): {content}"
        .to_string()
}

impl Default for AbstractionConfig {
    fn default() -> Self {
        AbstractionConfig {
            enabled: false,
            provider: default_abstraction_provider(),
            generate_overview: false,
            abstract_prompt_template: default_abstract_prompt_template(),
            overview_prompt_template: default_overview_prompt_template(),
            ollama_base_url: default_ollama_base_url(),
            ollama_model: default_abstraction_ollama_model(),
            openai_base_url: default_abstraction_openai_base_url(),
            openai_api_key: None,
            openai_model: default_abstraction_openai_model(),
            max_input_chars: default_abstraction_max_input_chars(),
            min_content_length: default_abstraction_min_content_length(),
        }
    }
}

/// Configuration for the health HTTP server (container lifecycle probes).
///
/// Provides /health and /status endpoints for orchestrators (Fly.io, Railway, k8s).
/// Runs on a separate port from the MCP stdio transport.
/// Nested env var overrides use double underscores:
///   MEMCP_HEALTH__PORT=9090
///   MEMCP_HEALTH__ENABLED=false
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Enable the health HTTP server (default: true in daemon mode)
    #[serde(default = "default_health_enabled")]
    pub enabled: bool,

    /// Port for the health HTTP server
    #[serde(default = "default_health_port")]
    pub port: u16,

    /// Bind address for the health HTTP server
    #[serde(default = "default_health_bind")]
    pub bind: String,
}

fn default_health_enabled() -> bool {
    true
}
fn default_health_port() -> u16 {
    9090
}
fn default_health_bind() -> String {
    "0.0.0.0".to_string()
}

impl Default for HealthConfig {
    fn default() -> Self {
        HealthConfig {
            enabled: default_health_enabled(),
            port: default_health_port(),
            bind: default_health_bind(),
        }
    }
}

/// Resource caps configuration for container deployments.
///
/// Controls resource limits for deployed instances. Used by /status endpoint
/// to surface current usage vs limits.
/// Nested env var overrides use double underscores:
///   MEMCP_RESOURCE_CAPS__MAX_MEMORIES=10000
///   MEMCP_RESOURCE_CAPS__MAX_DB_CONNECTIONS=5
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCapsConfig {
    /// Max number of live (non-deleted) memories. None = unlimited.
    #[serde(default)]
    pub max_memories: Option<u64>,

    /// Max batch size for embedding pipeline processing.
    #[serde(default = "default_max_embedding_batch_size")]
    pub max_embedding_batch_size: usize,

    /// Max search results returned per query.
    #[serde(default = "default_max_search_results")]
    pub max_search_results: i64,

    /// Max DB connection pool size.
    #[serde(default = "default_max_db_connections")]
    pub max_db_connections: u32,
}

fn default_max_embedding_batch_size() -> usize {
    64
}
fn default_max_search_results() -> i64 {
    100
}
fn default_max_db_connections() -> u32 {
    10
}

impl Default for ResourceCapsConfig {
    fn default() -> Self {
        ResourceCapsConfig {
            max_memories: None,
            max_embedding_batch_size: default_max_embedding_batch_size(),
            max_search_results: default_max_search_results(),
            max_db_connections: default_max_db_connections(),
        }
    }
}

/// Configuration for store operations.
///
/// Controls sync store timeout and other store-level behaviors.
/// Nested env var overrides use double underscores:
///   MEMCP_STORE__SYNC_TIMEOUT_SECS=10
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Timeout in seconds for sync store (`--wait`/`wait: true`).
    /// After this timeout, store returns success with `embedding_status: "pending"`.
    /// Default: 5 seconds (covers local fastembed ~200ms and OpenAI ~2s with margin).
    #[serde(default = "default_sync_timeout_secs")]
    pub sync_timeout_secs: u64,
}

fn default_sync_timeout_secs() -> u64 {
    5
}

impl Default for StoreConfig {
    fn default() -> Self {
        StoreConfig {
            sync_timeout_secs: default_sync_timeout_secs(),
        }
    }
}

/// Configuration for the AI brain curation subsystem.
///
/// Periodic self-maintenance daemon worker that reviews memories —
/// merges related entries, strengthens important ones, flags outdated ones.
/// Disabled by default — opt in via `[curation] enabled = true`.
/// Nested env var overrides use double underscores:
///   MEMCP_CURATION__ENABLED=true
///   MEMCP_CURATION__INTERVAL_SECS=86400
///   MEMCP_CURATION__LLM_PROVIDER=ollama
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurationConfig {
    /// Whether curation is enabled (default: false — opt-in)
    #[serde(default)]
    pub enabled: bool,

    /// Sensitivity level for injection detection (default: Medium — matches original behavior).
    /// Low = lenient (fewer flags), High = aggressive (more flags).
    #[serde(default)]
    pub sensitivity: crate::pipeline::curation::algorithmic::CurationSensitivity,

    /// How often to run curation in seconds (default: 86400 — daily)
    #[serde(default = "default_curation_interval_secs")]
    pub interval_secs: u64,

    /// Cosine similarity threshold for clustering related memories (default: 0.85).
    /// Lower than dedup (0.95) to catch paraphrases and topic-adjacent memories.
    #[serde(default = "default_curation_cluster_threshold")]
    pub cluster_similarity_threshold: f64,

    /// Stability threshold below which memories are candidates for stale flagging (default: 0.3)
    #[serde(default = "default_curation_stale_salience")]
    pub stale_salience_threshold: f64,

    /// Minimum age in days before a memory can be flagged stale (default: 30)
    #[serde(default = "default_curation_stale_age_days")]
    pub stale_age_days: u32,

    /// Stability value to set when flagging a memory as stale (default: 0.1).
    /// Very low but non-zero — effectively removes from search results.
    #[serde(default = "default_curation_stale_stability")]
    pub stale_stability_target: f64,

    /// Maximum merge operations per curation run (default: 20)
    #[serde(default = "default_curation_max_merges")]
    pub max_merges_per_run: usize,

    /// Maximum stale-flag operations per curation run (default: 50)
    #[serde(default = "default_curation_max_flags")]
    pub max_flags_per_run: usize,

    /// Maximum strengthen operations per curation run (default: 50)
    #[serde(default = "default_curation_max_strengthens")]
    pub max_strengthens_per_run: usize,

    /// Maximum candidate memories to process per run (default: 500).
    /// Prevents first-run full-corpus scan from overwhelming the system.
    #[serde(default = "default_curation_max_candidates")]
    pub max_candidates_per_run: usize,

    /// Maximum memories per merge group (default: 5, per CONTEXT.md locked decision)
    #[serde(default = "default_curation_max_merge_group")]
    pub max_merge_group_size: usize,

    /// LLM provider for curation: "ollama" or "openai".
    /// None = algorithmic-only mode (default — works without Ollama).
    #[serde(default)]
    pub llm_provider: Option<String>,

    /// Ollama server base URL
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    /// Ollama model for curation
    #[serde(default = "default_curation_ollama_model")]
    pub ollama_model: String,

    /// OpenAI-compatible base URL
    #[serde(default = "default_curation_openai_base_url")]
    pub openai_base_url: String,

    /// OpenAI-compatible API key — required when llm_provider = "openai"
    #[serde(default)]
    pub openai_api_key: Option<String>,

    /// OpenAI-compatible model for curation
    #[serde(default = "default_curation_openai_model")]
    pub openai_model: String,
}

fn default_curation_interval_secs() -> u64 {
    86400
}
fn default_curation_cluster_threshold() -> f64 {
    0.85
}
fn default_curation_stale_salience() -> f64 {
    0.3
}
fn default_curation_stale_age_days() -> u32 {
    30
}
fn default_curation_stale_stability() -> f64 {
    0.1
}
fn default_curation_max_merges() -> usize {
    20
}
fn default_curation_max_flags() -> usize {
    50
}
fn default_curation_max_strengthens() -> usize {
    50
}
fn default_curation_max_candidates() -> usize {
    500
}
fn default_curation_max_merge_group() -> usize {
    5
}
fn default_curation_ollama_model() -> String {
    "llama3.2:3b".to_string()
}
fn default_curation_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_curation_openai_model() -> String {
    "gpt-4o-mini".to_string()
}

impl Default for CurationConfig {
    fn default() -> Self {
        CurationConfig {
            enabled: false,
            interval_secs: default_curation_interval_secs(),
            cluster_similarity_threshold: default_curation_cluster_threshold(),
            stale_salience_threshold: default_curation_stale_salience(),
            stale_age_days: default_curation_stale_age_days(),
            stale_stability_target: default_curation_stale_stability(),
            max_merges_per_run: default_curation_max_merges(),
            max_flags_per_run: default_curation_max_flags(),
            max_strengthens_per_run: default_curation_max_strengthens(),
            max_candidates_per_run: default_curation_max_candidates(),
            max_merge_group_size: default_curation_max_merge_group(),
            llm_provider: None,
            ollama_base_url: default_ollama_base_url(),
            ollama_model: default_curation_ollama_model(),
            openai_base_url: default_curation_openai_base_url(),
            openai_api_key: None,
            openai_model: default_curation_openai_model(),
            sensitivity: Default::default(),
        }
    }
}

/// Configuration for resource limits and capacity thresholds.
///
/// Controls when to warn about approaching capacity and whether to auto-trigger GC.
/// Nested env var overrides:
///   MEMCP_RESOURCE_LIMITS__WARN_PERCENT=80
///   MEMCP_RESOURCE_LIMITS__AUTO_GC=true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitsConfig {
    /// Percentage of max_memories at which to start warning (default: 80)
    #[serde(default = "default_warn_percent")]
    pub warn_percent: u64,
    /// Percentage of max_memories at which to hard-reject stores (default: 110)
    #[serde(default = "default_hard_cap_percent")]
    pub hard_cap_percent: u64,
    /// Auto-trigger GC when above warn_percent (default: false — free tier off, paid tier on)
    #[serde(default)]
    pub auto_gc: bool,
    /// Minimum minutes between auto-GC runs (default: 15)
    #[serde(default = "default_auto_gc_cooldown_mins")]
    pub auto_gc_cooldown_mins: u64,
}

fn default_warn_percent() -> u64 {
    80
}
fn default_hard_cap_percent() -> u64 {
    110
}
fn default_auto_gc_cooldown_mins() -> u64 {
    15
}

impl Default for ResourceLimitsConfig {
    fn default() -> Self {
        ResourceLimitsConfig {
            warn_percent: default_warn_percent(),
            hard_cap_percent: default_hard_cap_percent(),
            auto_gc: false,
            auto_gc_cooldown_mins: default_auto_gc_cooldown_mins(),
        }
    }
}

/// Configuration for type-specific FSRS stability initialization.
///
/// Different memory types have different natural lifetimes. Architecture decisions
/// should persist much longer than ephemeral observations. By setting different
/// initial FSRS stability values per type_hint at store time, important memories
/// decay slower through the existing salience scoring system.
///
/// Nested env var overrides use double underscores:
///   MEMCP_RETENTION__DEFAULT_STABILITY=2.5
///
/// Override a specific type via TOML:
/// ```toml
/// [retention]
/// [retention.type_stability]
/// decision = 7.0
/// observation = 0.5
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    /// Map of type_hint → initial FSRS stability (days).
    /// Higher stability = slower salience decay.
    /// Default tiers: decision=5.0, preference=5.0, instruction=3.5, fact=2.5, observation=1.0, summary=2.0
    #[serde(default = "default_type_stability")]
    pub type_stability: HashMap<String, f64>,

    /// Default stability for untyped or unknown type_hint memories.
    /// Default: 2.5 (matches the previous global default stability)
    #[serde(default = "default_retention_stability")]
    pub default_stability: f64,
}

fn default_type_stability() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("decision".to_string(), 5.0);
    m.insert("preference".to_string(), 5.0);
    m.insert("instruction".to_string(), 3.5);
    m.insert("fact".to_string(), 2.5);
    m.insert("observation".to_string(), 1.0);
    m.insert("summary".to_string(), 2.0);
    m
}

fn default_retention_stability() -> f64 {
    2.5
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            type_stability: default_type_stability(),
            default_stability: default_retention_stability(),
        }
    }
}

impl RetentionConfig {
    /// Returns the initial FSRS stability for the given type_hint.
    ///
    /// Falls back to `default_stability` when the type is unknown or empty.
    /// Higher stability = slower salience decay over time.
    pub fn stability_for_type(&self, type_hint: &str) -> f64 {
        if type_hint.is_empty() {
            return self.default_stability;
        }
        self.type_stability
            .get(type_hint)
            .copied()
            .unwrap_or(self.default_stability)
    }
}

/// Configuration for the `memcp import` pipeline.
///
/// Applied during all import commands (jsonl, openclaw, chatgpt, etc.).
/// Noise patterns here are merged with per-source hardcoded patterns.
/// Nested env var overrides use double underscores:
///   MEMCP_IMPORT__BATCH_SIZE=50
///   MEMCP_IMPORT__DEFAULT_PROJECT=myproject
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportConfig {
    /// Custom noise patterns to apply during import (in addition to per-source defaults).
    /// Each pattern is a case-insensitive substring — matching content is dropped (noise).
    /// Example: ["CUSTOM_NOISE", "system heartbeat"]
    pub noise_patterns: Vec<String>,

    /// Default batch size for import database transactions (default: 100).
    /// CLI --batch-size flag overrides this value.
    pub batch_size: usize,

    /// Default project for imported memories (default: none).
    /// CLI --project flag overrides this value.
    pub default_project: Option<String>,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            noise_patterns: vec![],
            batch_size: 100,
            default_project: None,
        }
    }
}

/// Configuration for the retroactive neighbor enrichment daemon worker.
///
/// When enabled, the enrichment worker periodically scans for un-enriched memories,
/// finds their nearest neighbors, and uses an LLM to suggest tags that improve
/// discoverability based on neighbor relationships.
///
/// Disabled by default (opt-in via `enabled = true` in [enrichment] config section).
/// Uses the same Ollama/OpenAI provider as query intelligence — no separate LLM config needed.
///
/// Nested env var overrides use double underscores:
///   MEMCP_ENRICHMENT__ENABLED=true
///   MEMCP_ENRICHMENT__BATCH_LIMIT=100
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentConfig {
    /// Whether the enrichment daemon worker is enabled. Default: false (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Maximum memories to process per enrichment sweep. Default: 50.
    #[serde(default = "default_enrichment_batch_limit")]
    pub batch_limit: usize,

    /// Seconds between enrichment sweeps. Default: 3600 (1 hour).
    #[serde(default = "default_enrichment_sweep_interval")]
    pub sweep_interval_secs: u64,

    /// Number of nearest neighbors to consider when enriching a memory. Default: 5.
    #[serde(default = "default_enrichment_neighbor_depth")]
    pub neighbor_depth: usize,

    /// Minimum cosine similarity for a memory to be included as a neighbor. Default: 0.7.
    #[serde(default = "default_enrichment_similarity_threshold")]
    pub neighbor_similarity_threshold: f64,
}

fn default_enrichment_batch_limit() -> usize {
    50
}
fn default_enrichment_sweep_interval() -> u64 {
    3600
}
fn default_enrichment_neighbor_depth() -> usize {
    5
}
fn default_enrichment_similarity_threshold() -> f64 {
    0.7
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        EnrichmentConfig {
            enabled: false,
            batch_limit: default_enrichment_batch_limit(),
            sweep_interval_secs: default_enrichment_sweep_interval(),
            neighbor_depth: default_enrichment_neighbor_depth(),
            neighbor_similarity_threshold: default_enrichment_similarity_threshold(),
        }
    }
}

/// Configuration for HTTP API rate limiting.
///
/// Per-endpoint rate limits applied by tower_governor middleware.
/// Disabled by default — opt in via `[rate_limit] enabled = true`.
/// Nested env var overrides use double underscores:
///   MEMCP_RATE_LIMIT__ENABLED=true
///   MEMCP_RATE_LIMIT__GLOBAL_RPS=200
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Whether rate limiting is enabled (default: true)
    #[serde(default = "default_rate_limit_enabled")]
    pub enabled: bool,
    /// Global requests per second across all endpoints (default: 200)
    #[serde(default = "default_global_rps")]
    pub global_rps: u32,
    /// Requests per second for recall endpoint (default: 100)
    #[serde(default = "default_recall_rps")]
    pub recall_rps: u32,
    /// Requests per second for store endpoint (default: 50)
    #[serde(default = "default_store_rps")]
    pub store_rps: u32,
    /// Requests per second for search endpoint (default: 100)
    #[serde(default = "default_search_rps")]
    pub search_rps: u32,
    /// Requests per second for annotate endpoint (default: 50)
    #[serde(default = "default_annotate_rps")]
    pub annotate_rps: u32,
    /// Requests per second for update endpoint (default: 50)
    #[serde(default = "default_update_rps")]
    pub update_rps: u32,
    /// Burst multiplier over the base RPS (default: 2)
    #[serde(default = "default_burst_multiplier")]
    pub burst_multiplier: u32,
    /// Requests per second for discover endpoint (default: 50, compute-heavy)
    #[serde(default = "default_discover_rps")]
    pub discover_rps: u32,
    /// Requests per second for delete endpoint (default: 50)
    #[serde(default = "default_delete_rps")]
    pub delete_rps: u32,
    /// Requests per second for export endpoint (default: 10, bulk read)
    #[serde(default = "default_export_rps")]
    pub export_rps: u32,
}

fn default_rate_limit_enabled() -> bool {
    true
}
fn default_global_rps() -> u32 {
    200
}
fn default_recall_rps() -> u32 {
    100
}
fn default_store_rps() -> u32 {
    50
}
fn default_search_rps() -> u32 {
    100
}
fn default_annotate_rps() -> u32 {
    50
}
fn default_update_rps() -> u32 {
    50
}
fn default_burst_multiplier() -> u32 {
    2
}
fn default_discover_rps() -> u32 {
    50
}
fn default_delete_rps() -> u32 {
    50
}
fn default_export_rps() -> u32 {
    10
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        RateLimitConfig {
            enabled: default_rate_limit_enabled(),
            global_rps: default_global_rps(),
            recall_rps: default_recall_rps(),
            store_rps: default_store_rps(),
            search_rps: default_search_rps(),
            annotate_rps: default_annotate_rps(),
            update_rps: default_update_rps(),
            burst_multiplier: default_burst_multiplier(),
            discover_rps: default_discover_rps(),
            delete_rps: default_delete_rps(),
            export_rps: default_export_rps(),
        }
    }
}

/// Configuration for observability and metrics collection.
///
/// Controls Prometheus metrics exporter and pool polling interval.
/// Nested env var overrides use double underscores:
///   MEMCP_OBSERVABILITY__METRICS_ENABLED=false
///   MEMCP_OBSERVABILITY__POOL_POLL_INTERVAL_SECS=10
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Whether Prometheus metrics are enabled (default: true)
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,
    /// How often to poll DB connection pool stats in seconds (default: 10)
    #[serde(default = "default_pool_poll_interval_secs")]
    pub pool_poll_interval_secs: u64,
}

fn default_metrics_enabled() -> bool {
    true
}
fn default_pool_poll_interval_secs() -> u64 {
    10
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        ObservabilityConfig {
            metrics_enabled: default_metrics_enabled(),
            pool_poll_interval_secs: default_pool_poll_interval_secs(),
        }
    }
}

/// Configuration for secret and PII redaction on ingestion.
///
/// Secrets are enabled by default (detects API keys, tokens, etc.).
/// PII detection is opt-in (SSN, credit card). Email is never redacted.
/// Nested env var overrides use double underscores:
///   MEMCP_REDACTION__SECRETS_ENABLED=false
///   MEMCP_REDACTION__PII_ENABLED=true
///   MEMCP_REDACTION__ENTROPY_THRESHOLD=4.0
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionConfig {
    /// Whether secret detection is enabled (default: true)
    #[serde(default = "default_secrets_enabled")]
    pub secrets_enabled: bool,
    /// Whether PII detection is enabled (default: false, opt-in)
    #[serde(default)]
    pub pii_enabled: bool,
    /// Minimum Shannon entropy for generic secret detection (default: 3.5)
    #[serde(default = "default_entropy_threshold")]
    pub entropy_threshold: f64,
    /// Allowlist configuration — values and patterns that bypass redaction
    #[serde(default)]
    pub allowlist: AllowlistConfig,
    /// Custom redaction rules (user-provided patterns)
    #[serde(default)]
    pub custom_rules: Vec<CustomRuleConfig>,
}

/// Allowlist configuration for redaction bypass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AllowlistConfig {
    /// Exact string values that bypass redaction
    #[serde(default)]
    pub values: Vec<String>,
    /// Regex patterns — any match containing these bypasses redaction
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// A user-provided custom redaction rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRuleConfig {
    /// Regex pattern (must have a capture group for the secret value)
    pub pattern: String,
    /// Category name for the [REDACTED:category] marker
    pub category: String,
    /// Masking style: "partial" or "full" (default: "full")
    #[serde(default = "default_mask_style")]
    pub mask_style: String,
    /// Prefix length for partial masking
    #[serde(default)]
    pub prefix_len: Option<usize>,
}

fn default_secrets_enabled() -> bool {
    true
}

fn default_entropy_threshold() -> f64 {
    3.5
}

fn default_mask_style() -> String {
    "full".to_string()
}

impl Default for RedactionConfig {
    fn default() -> Self {
        RedactionConfig {
            secrets_enabled: default_secrets_enabled(),
            pii_enabled: false,
            entropy_threshold: default_entropy_threshold(),
            allowlist: AllowlistConfig::default(),
            custom_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Log level: trace, debug, info, warn, error
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Optional file path for log output (in addition to stderr)
    #[serde(default)]
    pub log_file: Option<String>,

    /// PostgreSQL database URL.
    /// Configurable via DATABASE_URL or MEMCP_DATABASE_URL env var, or database_url in memcp.toml.
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Embedding provider configuration.
    /// Existing configs without [embedding] section still work (serde default applied).
    #[serde(default)]
    pub embedding: EmbeddingConfig,

    /// Search subsystem configuration.
    /// Existing configs without [search] section still work (serde default applied).
    #[serde(default)]
    pub search: SearchConfig,

    /// Salience scoring configuration.
    /// Existing configs without [salience] section still work (serde default applied).
    #[serde(default)]
    pub salience: SalienceConfig,

    /// Extraction pipeline configuration.
    /// Existing configs without [extraction] section still work (serde default applied).
    #[serde(default)]
    pub extraction: ExtractionConfig,

    /// Memory consolidation configuration.
    /// Existing configs without [consolidation] section still work (serde default applied).
    #[serde(default)]
    pub consolidation: ConsolidationConfig,

    /// Query intelligence configuration (expansion + re-ranking).
    /// Existing configs without [query_intelligence] section still work (serde default applied).
    #[serde(default)]
    pub query_intelligence: QueryIntelligenceConfig,

    /// Auto-store sidecar configuration.
    /// Watches conversation log files and automatically ingests memories.
    /// Existing configs without [auto_store] section still work (serde default applied).
    #[serde(default)]
    pub auto_store: AutoStoreConfig,

    /// Content filtering configuration (topic exclusion).
    /// Existing configs without [content_filter] section still work (serde default applied).
    #[serde(default)]
    pub content_filter: ContentFilterConfig,

    /// Summarization configuration (for auto-store sidecar).
    /// Existing configs without [summarization] section still work (serde default applied).
    #[serde(default)]
    pub summarization: SummarizationConfig,

    /// Status line configuration (for Claude Code integration).
    /// Existing configs without [status_line] section still work (serde default applied).
    #[serde(default)]
    pub status_line: StatusLineConfig,

    /// Garbage collection configuration.
    /// Existing configs without [gc] section still work (serde default applied).
    #[serde(default)]
    pub gc: GcConfig,

    /// Semantic deduplication configuration.
    /// Existing configs without [dedup] section still work (serde default applied).
    #[serde(default)]
    pub dedup: DedupConfig,

    /// Idempotency configuration (content-hash dedup + caller-provided keys).
    /// Existing configs without [idempotency] section still work (serde default applied).
    #[serde(default)]
    pub idempotency: IdempotencyConfig,

    /// Recall configuration (automatic context injection).
    /// Existing configs without [recall] section still work (serde default applied).
    #[serde(default)]
    pub recall: RecallConfig,

    /// Health HTTP server configuration (container lifecycle probes).
    /// Existing configs without [health] section still work (serde default applied).
    #[serde(default)]
    pub health: HealthConfig,

    /// Resource caps configuration (container deployment limits).
    /// Existing configs without [resource_caps] section still work (serde default applied).
    #[serde(default)]
    pub resource_caps: ResourceCapsConfig,

    /// Memory chunking configuration.
    /// When enabled, long auto-store content is split into overlapping chunks.
    /// Existing configs without [chunking] section still work (serde default applied).
    #[serde(default)]
    pub chunking: ChunkingConfig,

    /// Store operation configuration (sync timeout, etc.)
    #[serde(default)]
    pub store: StoreConfig,

    /// Resource limits and capacity threshold configuration.
    #[serde(default)]
    pub resource_limits: ResourceLimitsConfig,

    /// AI brain curation configuration (periodic memory self-maintenance).
    /// When enabled, daemon periodically reviews memories — merging, strengthening, flagging stale.
    #[serde(default)]
    pub curation: CurationConfig,

    /// User-specific context for memory resolution (e.g., birth year for age-relative temporal refs).
    /// Existing configs without [user] section still work (serde default applied).
    #[serde(default)]
    pub user: UserConfig,

    /// Project scoping configuration.
    /// Existing configs with [workspace] section still work (serde alias applied).
    #[serde(default, alias = "workspace")]
    pub project: ProjectConfig,

    /// Temporal event time extraction configuration.
    /// Existing configs without [temporal] section still work (serde default applied).
    #[serde(default)]
    pub temporal: TemporalConfig,

    /// Import pipeline configuration.
    /// Controls noise patterns, batch size, and default project for `memcp import` commands.
    /// Existing configs without [import] section still work (serde default applied).
    #[serde(default)]
    pub import: ImportConfig,

    /// Type-specific FSRS stability initialization.
    /// Controls initial decay rate per memory type_hint at store time.
    /// Existing configs without [retention] section still work (serde default applied).
    #[serde(default)]
    pub retention: RetentionConfig,

    /// Retroactive neighbor enrichment configuration.
    /// Background daemon worker that adds tags to existing memories based on their nearest neighbors.
    /// Existing configs without [enrichment] section still work (serde default applied).
    #[serde(default)]
    pub enrichment: EnrichmentConfig,

    /// HTTP API rate limiting configuration.
    /// Controls per-endpoint request rate limits via tower_governor middleware.
    /// Existing configs without [rate_limit] section still work (serde default applied).
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Observability configuration (Prometheus metrics + pool polling).
    /// Existing configs without [observability] section still work (serde default applied).
    #[serde(default)]
    pub observability: ObservabilityConfig,

    /// Redaction configuration (secret and PII masking on ingestion).
    /// Secrets enabled by default, PII opt-in. Existing configs without [redaction] section still work.
    #[serde(default)]
    pub redaction: RedactionConfig,

    /// Tiered content abstraction configuration (L0 abstract + L1 overview generation).
    /// Disabled by default — opt in via `[abstraction] enabled = true`.
    /// Existing configs without [abstraction] section still work (serde default applied).
    #[serde(default)]
    pub abstraction: AbstractionConfig,

    /// Input size limits (content, tags, query, batch).
    /// Prevents resource exhaustion via oversized inputs at all transport layers.
    /// Existing configs without [input_limits] section still work (serde default applied).
    #[serde(default)]
    pub input_limits: crate::validation::InputLimitsConfig,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_database_url() -> String {
    "postgres://memcp:memcp@localhost:5432/memcp".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_level: default_log_level(),
            log_file: None,
            database_url: default_database_url(),
            embedding: EmbeddingConfig::default(),
            search: SearchConfig::default(),
            salience: SalienceConfig::default(),
            extraction: ExtractionConfig::default(),
            consolidation: ConsolidationConfig::default(),
            query_intelligence: QueryIntelligenceConfig::default(),
            auto_store: AutoStoreConfig::default(),
            content_filter: ContentFilterConfig::default(),
            summarization: SummarizationConfig::default(),
            status_line: StatusLineConfig::default(),
            gc: GcConfig::default(),
            dedup: DedupConfig::default(),
            idempotency: IdempotencyConfig::default(),
            recall: RecallConfig::default(),
            health: HealthConfig::default(),
            resource_caps: ResourceCapsConfig::default(),
            chunking: ChunkingConfig::default(),
            store: StoreConfig::default(),
            resource_limits: ResourceLimitsConfig::default(),
            curation: CurationConfig::default(),
            user: UserConfig::default(),
            project: ProjectConfig::default(),
            temporal: TemporalConfig::default(),
            import: ImportConfig::default(),
            retention: RetentionConfig::default(),
            enrichment: EnrichmentConfig::default(),
            rate_limit: RateLimitConfig::default(),
            observability: ObservabilityConfig::default(),
            redaction: RedactionConfig::default(),
            input_limits: crate::validation::InputLimitsConfig::default(),
            abstraction: AbstractionConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from defaults, TOML file, and environment variables
    ///
    /// Environment variables override TOML file values.
    /// DATABASE_URL is checked first (standard PostgreSQL convention),
    /// then MEMCP_DATABASE_URL, then database_url in memcp.toml.
    pub fn load() -> Result<Config, MemcpError> {
        let config: Config = Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::file("memcp.toml"))
            // Standard DATABASE_URL env var (highest priority for database config)
            .merge(
                Env::raw()
                    .only(&["DATABASE_URL"])
                    .map(|_| "database_url".into()),
            )
            // MEMCP_-prefixed env vars (includes MEMCP_DATABASE_URL, MEMCP_LOG_LEVEL, etc.)
            // Double underscore handles nested: MEMCP_EMBEDDING__PROVIDER=openai
            .merge(Env::prefixed("MEMCP_"))
            .extract()
            .map_err(|e| MemcpError::Config(format!("Failed to load config: {}", e)))?;

        // SEC-06: Validate all provider URLs against SSRF at config load time
        config.validate_provider_urls()?;

        Ok(config)
    }

    /// Validate all configurable provider URLs against SSRF.
    ///
    /// SEC-06: Checks extraction, query intelligence, summarization, temporal,
    /// curation, and embedding base URLs. Rejects dangerous schemes and private IPs.
    pub fn validate_provider_urls(&self) -> Result<(), MemcpError> {
        use crate::validation::validate_provider_url;
        let allow = self.input_limits.allow_localhost_http;

        // Extraction URLs
        validate_provider_url(&self.extraction.ollama_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("extraction.ollama_base_url: {}", e)))?;

        // Query intelligence URLs
        validate_provider_url(&self.query_intelligence.ollama_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("query_intelligence.ollama_base_url: {}", e)))?;
        validate_provider_url(&self.query_intelligence.openai_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("query_intelligence.openai_base_url: {}", e)))?;

        // Summarization URLs
        validate_provider_url(&self.summarization.ollama_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("summarization.ollama_base_url: {}", e)))?;
        validate_provider_url(&self.summarization.openai_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("summarization.openai_base_url: {}", e)))?;

        // Temporal URLs (optional)
        if let Some(ref url) = self.temporal.openai_base_url {
            validate_provider_url(url, allow)
                .map_err(|e| MemcpError::Config(format!("temporal.openai_base_url: {}", e)))?;
        }

        // Curation URLs
        validate_provider_url(&self.curation.ollama_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("curation.ollama_base_url: {}", e)))?;
        validate_provider_url(&self.curation.openai_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("curation.openai_base_url: {}", e)))?;

        // Embedding URL (optional)
        if let Some(ref url) = self.embedding.openai_base_url {
            validate_provider_url(url, allow)
                .map_err(|e| MemcpError::Config(format!("embedding.openai_base_url: {}", e)))?;
        }

        // Abstraction URLs
        validate_provider_url(&self.abstraction.ollama_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("abstraction.ollama_base_url: {}", e)))?;
        validate_provider_url(&self.abstraction.openai_base_url, allow)
            .map_err(|e| MemcpError::Config(format!("abstraction.openai_base_url: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_defaults() {
        let config = RetentionConfig::default();

        assert_eq!(
            config.stability_for_type("decision"),
            5.0,
            "decision should be 5.0"
        );
        assert_eq!(
            config.stability_for_type("preference"),
            5.0,
            "preference should be 5.0"
        );
        assert_eq!(
            config.stability_for_type("instruction"),
            3.5,
            "instruction should be 3.5"
        );
        assert_eq!(config.stability_for_type("fact"), 2.5, "fact should be 2.5");
        assert_eq!(
            config.stability_for_type("observation"),
            1.0,
            "observation should be 1.0"
        );
        assert_eq!(
            config.stability_for_type("summary"),
            2.0,
            "summary should be 2.0"
        );
    }

    #[test]
    fn test_retention_untyped() {
        let config = RetentionConfig::default();

        assert_eq!(
            config.stability_for_type(""),
            2.5,
            "empty string should return default 2.5"
        );
        assert_eq!(
            config.stability_for_type("unknown_type"),
            2.5,
            "unknown type should return default 2.5"
        );
        assert_eq!(
            config.stability_for_type("foobar"),
            2.5,
            "arbitrary type should return default 2.5"
        );
    }

    #[test]
    fn test_retention_serde() {
        let config = RetentionConfig::default();
        let json = serde_json::to_string(&config).expect("should serialize");
        let deserialized: RetentionConfig =
            serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(deserialized.default_stability, config.default_stability);
        assert_eq!(deserialized.stability_for_type("decision"), 5.0);
        assert_eq!(deserialized.stability_for_type("observation"), 1.0);
    }

    #[test]
    fn test_config_has_retention_field() {
        let config = Config::default();
        assert_eq!(config.retention.stability_for_type("decision"), 5.0);
        assert_eq!(config.retention.default_stability, 2.5);
    }

    #[test]
    fn test_enrichment_config_defaults() {
        let config = EnrichmentConfig::default();
        assert!(!config.enabled, "enrichment should be disabled by default");
        assert_eq!(config.batch_limit, 50, "batch_limit should default to 50");
        assert_eq!(
            config.sweep_interval_secs, 3600,
            "sweep_interval_secs should default to 3600"
        );
        assert_eq!(
            config.neighbor_depth, 5,
            "neighbor_depth should default to 5"
        );
        assert!(
            (config.neighbor_similarity_threshold - 0.7).abs() < f64::EPSILON,
            "neighbor_similarity_threshold should default to 0.7"
        );
    }

    #[test]
    fn test_config_has_enrichment_field() {
        let config = Config::default();
        assert!(
            !config.enrichment.enabled,
            "enrichment should be disabled by default"
        );
        assert_eq!(config.enrichment.batch_limit, 50);
        assert_eq!(config.enrichment.sweep_interval_secs, 3600);
        assert_eq!(config.enrichment.neighbor_depth, 5);
    }
}
