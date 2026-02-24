/// Configuration management using figment
///
/// Loads configuration with this precedence (highest wins):
/// 1. Defaults (hardcoded)
/// 2. TOML file: memcp.toml (in working directory)
/// 3. Environment variables: DATABASE_URL (standard PostgreSQL convention)
/// 4. Environment variables: prefixed MEMCP_ (e.g., MEMCP_LOG_LEVEL=debug)

use figment::{
    Figment,
    providers::{Env, Format, Toml, Serialized},
};
use serde::{Deserialize, Serialize};
use crate::errors::MemcpError;

/// Configuration for the search subsystem.
///
/// BM25 backend selection is explicit — having ParadeDB installed does NOT auto-switch.
/// Nested env var overrides use double underscores:
///   MEMCP_SEARCH__BM25_BACKEND=paradedb
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// BM25 backend: "native" (PostgreSQL tsvector, default) or "paradedb" (pg_search extension)
    /// Default: "native" — no extension required for self-hosted deployments
    #[serde(default = "default_bm25_backend")]
    pub bm25_backend: String,
}

fn default_bm25_backend() -> String {
    "native".to_string()
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            bm25_backend: default_bm25_backend(),
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

fn default_w_recency() -> f64 { 0.25 }
fn default_w_access() -> f64 { 0.15 }
fn default_w_semantic() -> f64 { 0.45 }
fn default_w_reinforce() -> f64 { 0.15 }
fn default_recency_lambda() -> f64 { 0.01 }

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

fn default_consolidation_enabled() -> bool { true }
fn default_similarity_threshold() -> f64 { 0.92 }
fn default_max_consolidation_group() -> usize { 5 }

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

    /// Maximum combined latency budget in ms (default: 2000)
    #[serde(default = "default_latency_budget_ms")]
    pub latency_budget_ms: u64,

    /// Max content chars sent to re-ranker per candidate (default: 500)
    #[serde(default = "default_rerank_content_chars")]
    pub rerank_content_chars: usize,
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
    "gpt-4o-mini".to_string()
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
            latency_budget_ms: default_latency_budget_ms(),
            rerank_content_chars: default_rerank_content_chars(),
        }
    }
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

    /// Override vector dimension. Auto-detected from model if omitted.
    /// Only needed for custom/unknown models.
    #[serde(default)]
    pub dimension: Option<usize>,
}

fn default_embedding_provider() -> String {
    "local".to_string()
}

fn default_cache_dir() -> String {
    dirs::cache_dir()
        .map(|p| p.join("memcp").join("models").to_string_lossy().into_owned())
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
            dimension: None,
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
}

fn default_category_filter_enabled() -> bool { true }
fn default_block_tool_narration() -> bool { true }

impl Default for CategoryFilterConfig {
    fn default() -> Self {
        CategoryFilterConfig {
            enabled: default_category_filter_enabled(),
            block_tool_narration: default_block_tool_narration(),
            tool_narration_patterns: Vec::new(),
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

fn default_auto_store_format() -> String { "claude-code".to_string() }
fn default_auto_store_filter_mode() -> String { "none".to_string() }
fn default_auto_store_filter_provider() -> String { "ollama".to_string() }
fn default_auto_store_filter_model() -> String { "llama3.2".to_string() }
fn default_auto_store_poll_interval() -> u64 { 5 }
fn default_auto_store_dedup_window() -> u64 { 300 }

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

fn default_gc_enabled() -> bool { true }
fn default_gc_salience_threshold() -> f64 { 0.3 }
fn default_gc_min_age_days() -> u32 { 30 }
fn default_gc_min_memory_floor() -> u64 { 100 }
fn default_gc_interval_secs() -> u64 { 3600 }
fn default_gc_hard_purge_grace_days() -> u32 { 30 }

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

fn default_summarization_provider() -> String { "ollama".to_string() }
fn default_summarization_ollama_model() -> String { "llama3.2:3b".to_string() }
fn default_summarization_openai_base_url() -> String { "https://api.openai.com/v1".to_string() }
fn default_summarization_openai_model() -> String { "gpt-4o-mini".to_string() }
fn default_summarization_max_input_chars() -> usize { 4000 }
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
     distinct topics, separate them with semicolons.".to_string()
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
        Figment::new()
            .merge(Serialized::defaults(Config::default()))
            .merge(Toml::file("memcp.toml"))
            // Standard DATABASE_URL env var (highest priority for database config)
            .merge(Env::raw().only(&["DATABASE_URL"]).map(|_| "database_url".into()))
            // MEMCP_-prefixed env vars (includes MEMCP_DATABASE_URL, MEMCP_LOG_LEVEL, etc.)
            // Double underscore handles nested: MEMCP_EMBEDDING__PROVIDER=openai
            .merge(Env::prefixed("MEMCP_"))
            .extract()
            .map_err(|e| MemcpError::Config(format!("Failed to load config: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = Config::default();
        assert_eq!(config.log_level, "info");
        assert_eq!(config.log_file, None);
        assert_eq!(config.database_url, "postgres://memcp:memcp@localhost:5432/memcp");
        assert_eq!(config.embedding.provider, "local");
        assert_eq!(config.embedding.openai_api_key, None);
        assert_eq!(config.search.bm25_backend, "native");
    }
}
