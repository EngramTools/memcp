# Configuration Reference

memcp uses [figment](https://docs.rs/figment) for layered configuration. Values are loaded in this priority order (highest wins):

1. **Environment variables** -- `MEMCP_` prefix, double underscore for nesting (e.g., `MEMCP_SEARCH__BM25_BACKEND=paradedb`)
2. **TOML file** -- `memcp.toml` in the working directory
3. **Built-in defaults** -- every field has a sensible default

Special case: `DATABASE_URL` (no prefix) is checked first for the database connection, then `MEMCP_DATABASE_URL`, then the TOML `database_url` key.

---

## Root Configuration

Top-level fields that don't belong to a subsystem section.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `log_level` | `MEMCP_LOG_LEVEL` | String | `"info"` | Log level: trace, debug, info, warn, error |
| `log_file` | `MEMCP_LOG_FILE` | String? | `null` | Optional file path for log output (in addition to stderr) |
| `database_url` | `DATABASE_URL` or `MEMCP_DATABASE_URL` | String | `"postgres://memcp:memcp@localhost:5432/memcp"` | PostgreSQL connection URL |

---

## `[embedding]`

Embedding provider configuration. Controls which model generates vector embeddings for memories.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `provider` | `MEMCP_EMBEDDING__PROVIDER` | String | `"local"` | Provider: `"local"` (fastembed) or `"openai"` |
| `openai_api_key` | `MEMCP_EMBEDDING__OPENAI_API_KEY` | String? | `null` | OpenAI API key (required when provider = "openai") |
| `cache_dir` | `MEMCP_EMBEDDING__CACHE_DIR` | String | Platform cache dir + `/memcp/models` | Directory for caching fastembed model weights |
| `local_model` | `MEMCP_EMBEDDING__LOCAL_MODEL` | String | `"AllMiniLML6V2"` | Fastembed model identifier (384 dimensions) |
| `openai_model` | `MEMCP_EMBEDDING__OPENAI_MODEL` | String | `"text-embedding-3-small"` | OpenAI embedding model (1536 dimensions) |
| `openai_base_url` | `MEMCP_EMBEDDING__OPENAI_BASE_URL` | String? | `null` | API base URL override for OpenAI-compatible providers (e.g., Google Gemini) |
| `dimension` | `MEMCP_EMBEDDING__DIMENSION` | usize? | `null` | Vector dimension override (auto-detected from model if omitted) |
| `reembed_on_tag_change` | `MEMCP_EMBEDDING__REEMBED_ON_TAG_CHANGE` | bool | `false` | Re-embed when only tags change. When false, tag-only updates skip re-embed |
| `tiers` | `MEMCP_EMBEDDING__TIERS` | Map | `{}` | Named embedding tiers for multi-model support (empty = single-model mode) |

### `[embedding.tiers.<name>]`

Each tier represents a different embedding model (e.g., fast local vs quality API).

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `provider` | -- | String | (required) | `"local"` or `"openai"` |
| `model` | -- | String? | `null` | Model name (fastembed identifier or OpenAI model name) |
| `openai_api_key` | -- | String? | `null` | API key override (falls back to top-level if not set) |
| `base_url` | -- | String? | `null` | API base URL override |
| `dimension` | -- | usize? | `null` | Vector dimension override |
| `routing` | -- | RoutingConfig? | `null` | Routing rules (when this tier is selected at store time) |
| `promotion` | -- | PromotionConfig? | `null` | Promotion rules (for sweep worker to upgrade from lower tier) |

### `[embedding.tiers.<name>.routing]`

All conditions must be met (AND logic). Omitted conditions are not checked.

| Key | Type | Default | Description |
|-|-|-|-|
| `min_stability` | f64? | `null` | Minimum stability score for this tier |
| `type_hints` | String[] | `[]` | Memory type_hints that should use this tier |
| `min_content_length` | usize? | `null` | Minimum content length (chars) for this tier |

### `[embedding.tiers.<name>.promotion]`

Controls the sweep worker that upgrades memories from a lower tier to a higher-quality tier.

| Key | Type | Default | Description |
|-|-|-|-|
| `min_reinforcements` | u32 | `3` | Minimum reinforcement count to promote |
| `min_stability` | f64 | `0.8` | Minimum stability score to promote |
| `sweep_interval_minutes` | u64 | `60` | Sweep interval in minutes |
| `batch_cap` | usize | `15` | Max promotions per sweep cycle |

---

## `[search]`

Search subsystem configuration. Controls BM25 backend selection and salience filtering defaults.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `bm25_backend` | `MEMCP_SEARCH__BM25_BACKEND` | String | `"native"` | BM25 backend: `"native"` (PostgreSQL tsvector) or `"paradedb"` (pg_search extension) |
| `default_min_salience` | `MEMCP_SEARCH__DEFAULT_MIN_SALIENCE` | f64? | `null` | Global default minimum salience score (0.0-1.0). Applied when caller omits min_salience |
| `salience_hint_mode` | `MEMCP_SEARCH__SALIENCE_HINT_MODE` | bool | `false` | When true, empty results filtered by salience include a hint explaining how many were below threshold |

---

## `[salience]`

Salience scoring weights. Controls how much each dimension contributes to the final salience score. All four weights should ideally sum to 1.0 (not automatically normalized).

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `w_recency` | `MEMCP_SALIENCE__W_RECENCY` | f64 | `0.25` | Weight for recency dimension |
| `w_access` | `MEMCP_SALIENCE__W_ACCESS` | f64 | `0.15` | Weight for access frequency dimension |
| `w_semantic` | `MEMCP_SALIENCE__W_SEMANTIC` | f64 | `0.45` | Weight for semantic relevance dimension |
| `w_reinforce` | `MEMCP_SALIENCE__W_REINFORCE` | f64 | `0.15` | Weight for reinforcement strength dimension |
| `recency_lambda` | `MEMCP_SALIENCE__RECENCY_LAMBDA` | f64 | `0.01` | Exponential recency decay rate (~70-day half-life) |
| `debug_scoring` | `MEMCP_SALIENCE__DEBUG_SCORING` | bool | `false` | Enable debug scoring output (shows dimension breakdown in results) |

---

## `[extraction]`

Extraction pipeline configuration. Controls the LLM-based metadata extraction from memory content (topics, entities, temporal references).

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `provider` | `MEMCP_EXTRACTION__PROVIDER` | String | `"ollama"` | Provider: `"ollama"` (local) or `"openai"` |
| `ollama_base_url` | `MEMCP_EXTRACTION__OLLAMA_BASE_URL` | String | `"http://localhost:11434"` | Ollama server base URL |
| `ollama_model` | `MEMCP_EXTRACTION__OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for extraction |
| `openai_api_key` | `MEMCP_EXTRACTION__OPENAI_API_KEY` | String? | `null` | OpenAI API key (required when provider = "openai") |
| `openai_model` | `MEMCP_EXTRACTION__OPENAI_MODEL` | String | `"gpt-4o-mini"` | OpenAI model for extraction |
| `enabled` | `MEMCP_EXTRACTION__ENABLED` | bool | `true` | Whether extraction is enabled. Set to false to skip entirely |
| `max_content_chars` | `MEMCP_EXTRACTION__MAX_CONTENT_CHARS` | usize | `1500` | Maximum content characters to send for extraction (truncated beyond this) |

---

## `[consolidation]`

Memory consolidation configuration. When enabled, new memories trigger a pgvector similarity check; if existing memories exceed the threshold, they are auto-merged via LLM synthesis.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_CONSOLIDATION__ENABLED` | bool | `true` | Whether consolidation is enabled |
| `similarity_threshold` | `MEMCP_CONSOLIDATION__SIMILARITY_THRESHOLD` | f64 | `0.92` | Cosine similarity threshold above which memories are merged (0.0-1.0) |
| `max_consolidation_group` | `MEMCP_CONSOLIDATION__MAX_CONSOLIDATION_GROUP` | usize | `5` | Maximum originals merged into a single consolidated memory |

---

## `[query_intelligence]`

Query intelligence configuration (expansion + re-ranking). Both are disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `expansion_enabled` | `MEMCP_QUERY_INTELLIGENCE__EXPANSION_ENABLED` | bool | `false` | Enable query expansion |
| `reranking_enabled` | `MEMCP_QUERY_INTELLIGENCE__RERANKING_ENABLED` | bool | `false` | Enable LLM re-ranking |
| `expansion_provider` | `MEMCP_QUERY_INTELLIGENCE__EXPANSION_PROVIDER` | String | `"ollama"` | Provider for expansion: `"ollama"` or `"openai"` |
| `reranking_provider` | `MEMCP_QUERY_INTELLIGENCE__RERANKING_PROVIDER` | String | `"ollama"` | Provider for reranking: `"ollama"` or `"openai"` |
| `ollama_base_url` | `MEMCP_QUERY_INTELLIGENCE__OLLAMA_BASE_URL` | String | `"http://localhost:11434"` | Ollama base URL |
| `expansion_ollama_model` | `MEMCP_QUERY_INTELLIGENCE__EXPANSION_OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for expansion |
| `reranking_ollama_model` | `MEMCP_QUERY_INTELLIGENCE__RERANKING_OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for reranking |
| `openai_base_url` | `MEMCP_QUERY_INTELLIGENCE__OPENAI_BASE_URL` | String | `"https://api.openai.com/v1"` | OpenAI-compatible base URL (supports Kimi, custom endpoints) |
| `openai_api_key` | `MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY` | String? | `null` | OpenAI-compatible API key |
| `expansion_openai_model` | `MEMCP_QUERY_INTELLIGENCE__EXPANSION_OPENAI_MODEL` | String | `"gpt-5-mini"` | OpenAI model for expansion |
| `reranking_openai_model` | `MEMCP_QUERY_INTELLIGENCE__RERANKING_OPENAI_MODEL` | String | `"gpt-5-mini"` | OpenAI model for reranking |
| `multi_query_enabled` | `MEMCP_QUERY_INTELLIGENCE__MULTI_QUERY_ENABLED` | bool | `true` | Enable multi-query decomposition (complex queries split into sub-queries merged via RRF) |
| `latency_budget_ms` | `MEMCP_QUERY_INTELLIGENCE__LATENCY_BUDGET_MS` | u64 | `2000` | Maximum combined latency budget in ms |
| `rerank_content_chars` | `MEMCP_QUERY_INTELLIGENCE__RERANK_CONTENT_CHARS` | usize | `500` | Max content chars sent to re-ranker per candidate |

---

## `[auto_store]`

Auto-store sidecar configuration. Watches conversation log files and automatically ingests memories without explicit store calls. Disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_AUTO_STORE__ENABLED` | bool | `false` | Whether auto-store is enabled (opt-in) |
| `watch_paths` | `MEMCP_AUTO_STORE__WATCH_PATHS` | String[] | `[]` | Paths to watch for new log entries (supports ~ expansion) |
| `format` | `MEMCP_AUTO_STORE__FORMAT` | String | `"claude-code"` | Log format: `"claude-code"` or `"generic-jsonl"` |
| `filter_mode` | `MEMCP_AUTO_STORE__FILTER_MODE` | String | `"none"` | Filter mode: `"llm"`, `"heuristic"`, `"category"`, or `"none"` |
| `filter_provider` | `MEMCP_AUTO_STORE__FILTER_PROVIDER` | String | `"ollama"` | LLM provider for filtering: `"ollama"` or `"openai"` |
| `filter_model` | `MEMCP_AUTO_STORE__FILTER_MODEL` | String | `"llama3.2"` | Model name for LLM filter |
| `poll_interval_secs` | `MEMCP_AUTO_STORE__POLL_INTERVAL_SECS` | u64 | `5` | Fallback poll interval in seconds |
| `dedup_window_secs` | `MEMCP_AUTO_STORE__DEDUP_WINDOW_SECS` | u64 | `300` | Dedup window -- identical content within this window is skipped |
| `category_filter` | -- | CategoryFilterConfig | (see below) | Category filter configuration (used when filter_mode = "category") |

### `[auto_store.category_filter]`

Controls the heuristic-based category filter that blocks tool narration while passing through valuable content.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__ENABLED` | bool | `true` | Whether category filtering is enabled |
| `block_tool_narration` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__BLOCK_TOOL_NARRATION` | bool | `true` | Block tool narration patterns ("Let me read...", "Now I'll edit...") |
| `tool_narration_patterns` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__TOOL_NARRATION_PATTERNS` | String[] | `[]` | Additional custom regex patterns to block (anchored at ^) |
| `category_actions` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__CATEGORY_ACTIONS` | Map | (see below) | Per-category actions: `"store"`, `"skip"`, or `"store-low"` |
| `llm_provider` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__LLM_PROVIDER` | String? | `null` | LLM provider for classification (`"ollama"` or `"openai"`). None = heuristic-only |
| `llm_model` | `MEMCP_AUTO_STORE__CATEGORY_FILTER__LLM_MODEL` | String? | `null` | LLM model for classification |

**Default category actions:**

| Category | Action |
|-|-|
| `decision` | `store` |
| `preference` | `store` |
| `architecture` | `store` |
| `fact` | `store` |
| `instruction` | `store` |
| `correction` | `store` |
| `tool-narration` | `skip` |
| `ephemeral` | `skip` |
| `code-output` | `store-low` |
| `error-trace` | `store-low` |

---

## `[recall]`

Recall configuration for automatic context injection at session start.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `max_memories` | `MEMCP_RECALL__MAX_MEMORIES` | usize | `3` | Maximum memories returned per recall |
| `min_relevance` | `MEMCP_RECALL__MIN_RELEVANCE` | f64 | `0.7` | Minimum relevance threshold (0.0-1.0) |
| `session_idle_secs` | `MEMCP_RECALL__SESSION_IDLE_SECS` | u64 | `86400` | Session idle expiry in seconds (24 hours) |
| `bump_multiplier` | `MEMCP_RECALL__BUMP_MULTIPLIER` | f64 | `0.15` | Stability multiplier for recall salience bump (stability *= 1.0 + this) |
| `stability_ceiling` | `MEMCP_RECALL__STABILITY_CEILING` | f64 | `100.0` | Maximum stability value -- recall bump stops here |
| `truncation_chars` | `MEMCP_RECALL__TRUNCATION_CHARS` | usize | `200` | Max characters per memory in recall output (truncated with "...") |
| `preamble_override` | `MEMCP_RECALL__PREAMBLE_OVERRIDE` | String? | `null` | Custom preamble text for `recall --first` output |
| `related_context_enabled` | `MEMCP_RECALL__RELATED_CONTEXT_ENABLED` | bool | `true` | Show related_count and search hint per recalled memory |
| `tag_boost_weight` | `MEMCP_RECALL__TAG_BOOST_WEIGHT` | f64 | `0.1` | Weight per matching boost tag (additive per match) |
| `session_boost_weight` | `MEMCP_RECALL__SESSION_BOOST_WEIGHT` | f64 | `0.05` | Weight per matching session-accumulated tag |
| `tag_boost_cap` | `MEMCP_RECALL__TAG_BOOST_CAP` | f64 | `0.3` | Maximum total explicit tag boost |
| `session_boost_cap` | `MEMCP_RECALL__SESSION_BOOST_CAP` | f64 | `0.15` | Maximum total session tag boost |
| `session_topic_tracking` | `MEMCP_RECALL__SESSION_TOPIC_TRACKING` | bool | `true` | Cache recalled memory tags on session for implicit boosting |

---

## `[gc]`

Garbage collection configuration. Prunes low-salience, aged-out memories on a schedule.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_GC__ENABLED` | bool | `true` | Whether GC is enabled |
| `salience_threshold` | `MEMCP_GC__SALIENCE_THRESHOLD` | f64 | `0.3` | FSRS stability threshold below which memories are candidates for pruning |
| `min_age_days` | `MEMCP_GC__MIN_AGE_DAYS` | u32 | `30` | Minimum memory age in days before pruning |
| `min_memory_floor` | `MEMCP_GC__MIN_MEMORY_FLOOR` | u64 | `100` | Minimum live memories to retain (GC never prunes below this) |
| `gc_interval_secs` | `MEMCP_GC__GC_INTERVAL_SECS` | u64 | `3600` | How often to run GC in seconds (1 hour) |
| `hard_purge_grace_days` | `MEMCP_GC__HARD_PURGE_GRACE_DAYS` | u32 | `30` | Days after soft-delete before hard purge |

---

## `[curation]`

AI brain curation configuration. Periodic self-maintenance that merges related memories, strengthens important ones, and flags stale ones. Disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_CURATION__ENABLED` | bool | `false` | Whether curation is enabled (opt-in) |
| `interval_secs` | `MEMCP_CURATION__INTERVAL_SECS` | u64 | `86400` | How often to run curation in seconds (daily) |
| `cluster_similarity_threshold` | `MEMCP_CURATION__CLUSTER_SIMILARITY_THRESHOLD` | f64 | `0.85` | Cosine similarity threshold for clustering related memories |
| `stale_salience_threshold` | `MEMCP_CURATION__STALE_SALIENCE_THRESHOLD` | f64 | `0.3` | Stability threshold below which memories are candidates for stale flagging |
| `stale_age_days` | `MEMCP_CURATION__STALE_AGE_DAYS` | u32 | `30` | Minimum age in days before stale flagging |
| `stale_stability_target` | `MEMCP_CURATION__STALE_STABILITY_TARGET` | f64 | `0.1` | Stability value set when flagging stale (very low, effectively hidden) |
| `max_merges_per_run` | `MEMCP_CURATION__MAX_MERGES_PER_RUN` | usize | `20` | Maximum merge operations per curation run |
| `max_flags_per_run` | `MEMCP_CURATION__MAX_FLAGS_PER_RUN` | usize | `50` | Maximum stale-flag operations per run |
| `max_strengthens_per_run` | `MEMCP_CURATION__MAX_STRENGTHENS_PER_RUN` | usize | `50` | Maximum strengthen operations per run |
| `max_candidates_per_run` | `MEMCP_CURATION__MAX_CANDIDATES_PER_RUN` | usize | `500` | Maximum candidate memories to process per run |
| `max_merge_group_size` | `MEMCP_CURATION__MAX_MERGE_GROUP_SIZE` | usize | `5` | Maximum memories per merge group |
| `llm_provider` | `MEMCP_CURATION__LLM_PROVIDER` | String? | `null` | LLM provider: `"ollama"` or `"openai"`. None = algorithmic-only mode |
| `ollama_base_url` | `MEMCP_CURATION__OLLAMA_BASE_URL` | String | `"http://localhost:11434"` | Ollama server base URL |
| `ollama_model` | `MEMCP_CURATION__OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for curation |
| `openai_base_url` | `MEMCP_CURATION__OPENAI_BASE_URL` | String | `"https://api.openai.com/v1"` | OpenAI-compatible base URL |
| `openai_api_key` | `MEMCP_CURATION__OPENAI_API_KEY` | String? | `null` | OpenAI-compatible API key |
| `openai_model` | `MEMCP_CURATION__OPENAI_MODEL` | String | `"gpt-4o-mini"` | OpenAI-compatible model for curation |

---

## `[dedup]`

Semantic deduplication configuration. After embedding, the dedup worker checks similarity against existing memories. Near-duplicates above the threshold are merged. Async, fail-open.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_DEDUP__ENABLED` | bool | `true` | Whether semantic dedup is enabled |
| `similarity_threshold` | `MEMCP_DEDUP__SIMILARITY_THRESHOLD` | f64 | `0.95` | Cosine similarity threshold for near-duplicate detection (stricter than consolidation) |

---

## `[chunking]`

Memory chunking configuration. Splits long auto-store content into overlapping sentence-grouped chunks for better retrieval. Only affects auto-store ingestion.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_CHUNKING__ENABLED` | bool | `true` | Enable chunking for auto-store content |
| `max_chunk_chars` | `MEMCP_CHUNKING__MAX_CHUNK_CHARS` | usize | `1024` | Maximum characters per chunk (~256 tokens) |
| `overlap_sentences` | `MEMCP_CHUNKING__OVERLAP_SENTENCES` | usize | `2` | Number of sentences to overlap between adjacent chunks |
| `min_content_chars` | `MEMCP_CHUNKING__MIN_CONTENT_CHARS` | usize | `2048` | Minimum content length to trigger chunking (~512 tokens) |

---

## `[content_filter]`

Content filtering (topic exclusion). Two-tier system: regex patterns (fast) and semantic topic exclusion (embedding-based). Disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_CONTENT_FILTER__ENABLED` | bool | `false` | Whether content filtering is enabled (opt-in) |
| `default_action` | `MEMCP_CONTENT_FILTER__DEFAULT_ACTION` | String | `"drop"` | Action when content matches: `"drop"` (silent) or `"reject"` (return error) |
| `regex_patterns` | `MEMCP_CONTENT_FILTER__REGEX_PATTERNS` | String[] | `[]` | Regex patterns -- content matching ANY pattern is excluded |
| `excluded_topics` | `MEMCP_CONTENT_FILTER__EXCLUDED_TOPICS` | String[] | `[]` | Semantic topics to exclude (embedded at startup, checked via cosine similarity) |
| `semantic_threshold` | `MEMCP_CONTENT_FILTER__SEMANTIC_THRESHOLD` | f64 | `0.85` | Cosine similarity threshold for semantic exclusion |

---

## `[summarization]`

Auto-summarization configuration. When enabled, the auto-store sidecar summarizes AI assistant responses before storing. Disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_SUMMARIZATION__ENABLED` | bool | `false` | Whether summarization is enabled (opt-in) |
| `provider` | `MEMCP_SUMMARIZATION__PROVIDER` | String | `"ollama"` | Provider: `"ollama"` or `"openai"` (any OpenAI-compatible API) |
| `ollama_base_url` | `MEMCP_SUMMARIZATION__OLLAMA_BASE_URL` | String | `"http://localhost:11434"` | Ollama server base URL |
| `ollama_model` | `MEMCP_SUMMARIZATION__OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for summarization |
| `openai_base_url` | `MEMCP_SUMMARIZATION__OPENAI_BASE_URL` | String | `"https://api.openai.com/v1"` | OpenAI-compatible base URL (supports Kimi/Moonshot, local vLLM, etc.) |
| `openai_api_key` | `MEMCP_SUMMARIZATION__OPENAI_API_KEY` | String? | `null` | OpenAI-compatible API key |
| `openai_model` | `MEMCP_SUMMARIZATION__OPENAI_MODEL` | String | `"gpt-4o-mini"` | OpenAI-compatible model |
| `max_input_chars` | `MEMCP_SUMMARIZATION__MAX_INPUT_CHARS` | usize | `4000` | Maximum input characters before truncation |
| `prompt_template` | `MEMCP_SUMMARIZATION__PROMPT_TEMPLATE` | String | (built-in prompt) | System prompt template for summarization |

---

## `[idempotency]`

Idempotency configuration. Controls content-hash dedup window and caller-provided idempotency key behavior.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `dedup_window_secs` | `MEMCP_IDEMPOTENCY__DEDUP_WINDOW_SECS` | u64 | `60` | Time window for content-hash dedup (set to 0 to disable) |
| `key_ttl_secs` | `MEMCP_IDEMPOTENCY__KEY_TTL_SECS` | u64 | `86400` | TTL for idempotency keys (24 hours). Keys older than this are cleaned by GC |
| `max_key_length` | `MEMCP_IDEMPOTENCY__MAX_KEY_LENGTH` | usize | `256` | Maximum allowed idempotency key length in bytes |

---

## `[temporal]`

Temporal event time extraction configuration. Controls whether an LLM background worker extracts event_time from memory content. Regex-based extraction always runs inline.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `llm_enabled` | `MEMCP_TEMPORAL__LLM_ENABLED` | bool | `false` | Enable LLM background worker for subtle temporal extraction |
| `provider` | `MEMCP_TEMPORAL__PROVIDER` | String | `"ollama"` | Provider: `"ollama"` or `"openai"` |
| `ollama_model` | `MEMCP_TEMPORAL__OLLAMA_MODEL` | String | `"llama3.2:3b"` | Ollama model for temporal extraction |
| `openai_model` | `MEMCP_TEMPORAL__OPENAI_MODEL` | String | `"gpt-4o-mini"` | OpenAI model for temporal extraction |
| `openai_api_key` | `MEMCP_TEMPORAL__OPENAI_API_KEY` | String? | `null` | OpenAI-compatible API key |
| `openai_base_url` | `MEMCP_TEMPORAL__OPENAI_BASE_URL` | String? | `null` | OpenAI-compatible base URL |

---

## `[retention]`

Type-specific FSRS stability initialization. Different memory types decay at different rates. Higher stability = slower salience decay.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `default_stability` | `MEMCP_RETENTION__DEFAULT_STABILITY` | f64 | `2.5` | Default stability for untyped or unknown type_hint memories (days) |
| `type_stability` | -- | Map | (see below) | Map of type_hint to initial FSRS stability (days) |

**Default type stability values:**

| Type Hint | Stability (days) |
|-|-|
| `decision` | `5.0` |
| `preference` | `5.0` |
| `instruction` | `3.5` |
| `fact` | `2.5` |
| `observation` | `1.0` |
| `summary` | `2.0` |

---

## `[user]`

User-specific context for memory resolution.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `birth_year` | `MEMCP_USER__BIRTH_YEAR` | u32? | `null` | User's birth year for resolving relative-age references (e.g., "when I was 6") |

---

## `[project]`

Project scoping configuration. Projects isolate memories by codebase or context. Activation precedence: CLI flag > env var > config default.

Note: `[workspace]` is accepted as an alias for backwards compatibility.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `default_project` | `MEMCP_PROJECT__DEFAULT_PROJECT` | String? | `null` | Default project applied when no CLI flag or env var is set. None = global |

---

## `[store]`

Store operation configuration.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `sync_timeout_secs` | `MEMCP_STORE__SYNC_TIMEOUT_SECS` | u64 | `5` | Timeout for sync store (`--wait`). After timeout, returns success with `embedding_status: "pending"` |

---

## `[import]`

Import pipeline configuration. Applied during all `memcp import` commands.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `noise_patterns` | `MEMCP_IMPORT__NOISE_PATTERNS` | String[] | `[]` | Custom noise patterns (case-insensitive substrings) to drop during import |
| `batch_size` | `MEMCP_IMPORT__BATCH_SIZE` | usize | `100` | Default batch size for import DB transactions (CLI --batch-size overrides) |
| `default_project` | `MEMCP_IMPORT__DEFAULT_PROJECT` | String? | `null` | Default project for imported memories (CLI --project overrides) |

---

## `[enrichment]`

Retroactive neighbor enrichment daemon worker configuration. Scans for un-enriched memories, finds nearest neighbors, and uses an LLM to suggest tags. Disabled by default.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_ENRICHMENT__ENABLED` | bool | `false` | Whether the enrichment worker is enabled (opt-in) |
| `batch_limit` | `MEMCP_ENRICHMENT__BATCH_LIMIT` | usize | `50` | Maximum memories to process per enrichment sweep |
| `sweep_interval_secs` | `MEMCP_ENRICHMENT__SWEEP_INTERVAL_SECS` | u64 | `3600` | Seconds between enrichment sweeps (1 hour) |
| `neighbor_depth` | `MEMCP_ENRICHMENT__NEIGHBOR_DEPTH` | usize | `5` | Number of nearest neighbors to consider |
| `neighbor_similarity_threshold` | `MEMCP_ENRICHMENT__NEIGHBOR_SIMILARITY_THRESHOLD` | f64 | `0.7` | Minimum cosine similarity for neighbor inclusion |

---

## `[health]`

Health HTTP server configuration for container lifecycle probes. Provides `/health` and `/status` endpoints.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_HEALTH__ENABLED` | bool | `true` | Enable the health HTTP server |
| `port` | `MEMCP_HEALTH__PORT` | u16 | `9090` | Port for the health HTTP server |
| `bind` | `MEMCP_HEALTH__BIND` | String | `"0.0.0.0"` | Bind address for the health HTTP server |

---

## `[resource_caps]`

Resource caps for container deployments. Surfaced by the `/status` endpoint.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `max_memories` | `MEMCP_RESOURCE_CAPS__MAX_MEMORIES` | u64? | `null` | Max number of live (non-deleted) memories. None = unlimited |
| `max_embedding_batch_size` | `MEMCP_RESOURCE_CAPS__MAX_EMBEDDING_BATCH_SIZE` | usize | `64` | Max batch size for embedding pipeline |
| `max_search_results` | `MEMCP_RESOURCE_CAPS__MAX_SEARCH_RESULTS` | i64 | `100` | Max search results per query |
| `max_db_connections` | `MEMCP_RESOURCE_CAPS__MAX_DB_CONNECTIONS` | u32 | `10` | Max DB connection pool size |

---

## `[resource_limits]`

Resource limits and capacity thresholds. Controls warnings and auto-GC triggers.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `warn_percent` | `MEMCP_RESOURCE_LIMITS__WARN_PERCENT` | u64 | `80` | Percentage of max_memories at which to start warning |
| `hard_cap_percent` | `MEMCP_RESOURCE_LIMITS__HARD_CAP_PERCENT` | u64 | `110` | Percentage of max_memories at which to hard-reject stores |
| `auto_gc` | `MEMCP_RESOURCE_LIMITS__AUTO_GC` | bool | `false` | Auto-trigger GC when above warn_percent |
| `auto_gc_cooldown_mins` | `MEMCP_RESOURCE_LIMITS__AUTO_GC_COOLDOWN_MINS` | u64 | `15` | Minimum minutes between auto-GC runs |

---

## `[status_line]`

Claude Code status line integration configuration.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `format` | `MEMCP_STATUS_LINE__FORMAT` | String | `"ingest"` | Format: `"ingest"`, `"pending"`, or `"state"` |

---

## `[rate_limit]`

HTTP API rate limiting via tower_governor middleware.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `enabled` | `MEMCP_RATE_LIMIT__ENABLED` | bool | `true` | Whether rate limiting is enabled |
| `global_rps` | `MEMCP_RATE_LIMIT__GLOBAL_RPS` | u32 | `200` | Global requests per second across all endpoints |
| `recall_rps` | `MEMCP_RATE_LIMIT__RECALL_RPS` | u32 | `100` | Requests per second for recall endpoint |
| `store_rps` | `MEMCP_RATE_LIMIT__STORE_RPS` | u32 | `50` | Requests per second for store endpoint |
| `search_rps` | `MEMCP_RATE_LIMIT__SEARCH_RPS` | u32 | `100` | Requests per second for search endpoint |
| `annotate_rps` | `MEMCP_RATE_LIMIT__ANNOTATE_RPS` | u32 | `50` | Requests per second for annotate endpoint |
| `update_rps` | `MEMCP_RATE_LIMIT__UPDATE_RPS` | u32 | `50` | Requests per second for update endpoint |
| `burst_multiplier` | `MEMCP_RATE_LIMIT__BURST_MULTIPLIER` | u32 | `2` | Burst multiplier over the base RPS |
| `discover_rps` | `MEMCP_RATE_LIMIT__DISCOVER_RPS` | u32 | `50` | Requests per second for discover endpoint |
| `delete_rps` | `MEMCP_RATE_LIMIT__DELETE_RPS` | u32 | `50` | Requests per second for delete endpoint |
| `export_rps` | `MEMCP_RATE_LIMIT__EXPORT_RPS` | u32 | `10` | Requests per second for export endpoint (bulk read) |

---

## `[observability]`

Observability and metrics collection configuration.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `metrics_enabled` | `MEMCP_OBSERVABILITY__METRICS_ENABLED` | bool | `true` | Whether Prometheus metrics are enabled |
| `pool_poll_interval_secs` | `MEMCP_OBSERVABILITY__POOL_POLL_INTERVAL_SECS` | u64 | `10` | How often to poll DB connection pool stats in seconds |

---

## `[redaction]`

Secret and PII redaction on ingestion. Secrets are enabled by default. PII detection (SSN, credit card) is opt-in. Email is never redacted.

| Key | Env Var | Type | Default | Description |
|-|-|-|-|-|
| `secrets_enabled` | `MEMCP_REDACTION__SECRETS_ENABLED` | bool | `true` | Whether secret detection is enabled |
| `pii_enabled` | `MEMCP_REDACTION__PII_ENABLED` | bool | `false` | Whether PII detection is enabled (opt-in) |
| `entropy_threshold` | `MEMCP_REDACTION__ENTROPY_THRESHOLD` | f64 | `3.5` | Minimum Shannon entropy for generic secret detection |
| `allowlist` | -- | AllowlistConfig | (see below) | Values and patterns that bypass redaction |
| `custom_rules` | -- | CustomRuleConfig[] | `[]` | User-provided custom redaction rules |

### `[redaction.allowlist]`

| Key | Type | Default | Description |
|-|-|-|-|
| `values` | String[] | `[]` | Exact string values that bypass redaction |
| `patterns` | String[] | `[]` | Regex patterns -- any match containing these bypasses redaction |

### `[[redaction.custom_rules]]`

| Key | Type | Default | Description |
|-|-|-|-|
| `pattern` | String | (required) | Regex pattern (must have a capture group for the secret value) |
| `category` | String | (required) | Category name for the `[REDACTED:category]` marker |
| `mask_style` | String | `"full"` | Masking style: `"partial"` or `"full"` |
| `prefix_len` | usize? | `null` | Prefix length for partial masking |

---

## Appendix: Environment Variable Quick Reference

All environment variables sorted alphabetically. Use `MEMCP_` prefix with double underscores for nested keys.

| Env Var | Section | Default |
|-|-|-|
| `DATABASE_URL` | root | `postgres://memcp:memcp@localhost:5432/memcp` |
| `MEMCP_AUTO_STORE__CATEGORY_FILTER__BLOCK_TOOL_NARRATION` | auto_store.category_filter | `true` |
| `MEMCP_AUTO_STORE__CATEGORY_FILTER__ENABLED` | auto_store.category_filter | `true` |
| `MEMCP_AUTO_STORE__CATEGORY_FILTER__LLM_MODEL` | auto_store.category_filter | `null` |
| `MEMCP_AUTO_STORE__CATEGORY_FILTER__LLM_PROVIDER` | auto_store.category_filter | `null` |
| `MEMCP_AUTO_STORE__DEDUP_WINDOW_SECS` | auto_store | `300` |
| `MEMCP_AUTO_STORE__ENABLED` | auto_store | `false` |
| `MEMCP_AUTO_STORE__FILTER_MODE` | auto_store | `"none"` |
| `MEMCP_AUTO_STORE__FILTER_MODEL` | auto_store | `"llama3.2"` |
| `MEMCP_AUTO_STORE__FILTER_PROVIDER` | auto_store | `"ollama"` |
| `MEMCP_AUTO_STORE__FORMAT` | auto_store | `"claude-code"` |
| `MEMCP_AUTO_STORE__POLL_INTERVAL_SECS` | auto_store | `5` |
| `MEMCP_AUTO_STORE__WATCH_PATHS` | auto_store | `[]` |
| `MEMCP_CHUNKING__ENABLED` | chunking | `true` |
| `MEMCP_CHUNKING__MAX_CHUNK_CHARS` | chunking | `1024` |
| `MEMCP_CHUNKING__MIN_CONTENT_CHARS` | chunking | `2048` |
| `MEMCP_CHUNKING__OVERLAP_SENTENCES` | chunking | `2` |
| `MEMCP_CONSOLIDATION__ENABLED` | consolidation | `true` |
| `MEMCP_CONSOLIDATION__MAX_CONSOLIDATION_GROUP` | consolidation | `5` |
| `MEMCP_CONSOLIDATION__SIMILARITY_THRESHOLD` | consolidation | `0.92` |
| `MEMCP_CONTENT_FILTER__DEFAULT_ACTION` | content_filter | `"drop"` |
| `MEMCP_CONTENT_FILTER__ENABLED` | content_filter | `false` |
| `MEMCP_CONTENT_FILTER__SEMANTIC_THRESHOLD` | content_filter | `0.85` |
| `MEMCP_CURATION__CLUSTER_SIMILARITY_THRESHOLD` | curation | `0.85` |
| `MEMCP_CURATION__ENABLED` | curation | `false` |
| `MEMCP_CURATION__INTERVAL_SECS` | curation | `86400` |
| `MEMCP_CURATION__LLM_PROVIDER` | curation | `null` |
| `MEMCP_CURATION__MAX_CANDIDATES_PER_RUN` | curation | `500` |
| `MEMCP_CURATION__MAX_FLAGS_PER_RUN` | curation | `50` |
| `MEMCP_CURATION__MAX_MERGE_GROUP_SIZE` | curation | `5` |
| `MEMCP_CURATION__MAX_MERGES_PER_RUN` | curation | `20` |
| `MEMCP_CURATION__MAX_STRENGTHENS_PER_RUN` | curation | `50` |
| `MEMCP_CURATION__OLLAMA_BASE_URL` | curation | `"http://localhost:11434"` |
| `MEMCP_CURATION__OLLAMA_MODEL` | curation | `"llama3.2:3b"` |
| `MEMCP_CURATION__OPENAI_API_KEY` | curation | `null` |
| `MEMCP_CURATION__OPENAI_BASE_URL` | curation | `"https://api.openai.com/v1"` |
| `MEMCP_CURATION__OPENAI_MODEL` | curation | `"gpt-4o-mini"` |
| `MEMCP_CURATION__STALE_AGE_DAYS` | curation | `30` |
| `MEMCP_CURATION__STALE_SALIENCE_THRESHOLD` | curation | `0.3` |
| `MEMCP_CURATION__STALE_STABILITY_TARGET` | curation | `0.1` |
| `MEMCP_DATABASE_URL` | root | `postgres://memcp:memcp@localhost:5432/memcp` |
| `MEMCP_DEDUP__ENABLED` | dedup | `true` |
| `MEMCP_DEDUP__SIMILARITY_THRESHOLD` | dedup | `0.95` |
| `MEMCP_EMBEDDING__CACHE_DIR` | embedding | Platform-specific |
| `MEMCP_EMBEDDING__DIMENSION` | embedding | `null` |
| `MEMCP_EMBEDDING__LOCAL_MODEL` | embedding | `"AllMiniLML6V2"` |
| `MEMCP_EMBEDDING__OPENAI_API_KEY` | embedding | `null` |
| `MEMCP_EMBEDDING__OPENAI_BASE_URL` | embedding | `null` |
| `MEMCP_EMBEDDING__OPENAI_MODEL` | embedding | `"text-embedding-3-small"` |
| `MEMCP_EMBEDDING__PROVIDER` | embedding | `"local"` |
| `MEMCP_EMBEDDING__REEMBED_ON_TAG_CHANGE` | embedding | `false` |
| `MEMCP_ENRICHMENT__BATCH_LIMIT` | enrichment | `50` |
| `MEMCP_ENRICHMENT__ENABLED` | enrichment | `false` |
| `MEMCP_ENRICHMENT__NEIGHBOR_DEPTH` | enrichment | `5` |
| `MEMCP_ENRICHMENT__NEIGHBOR_SIMILARITY_THRESHOLD` | enrichment | `0.7` |
| `MEMCP_ENRICHMENT__SWEEP_INTERVAL_SECS` | enrichment | `3600` |
| `MEMCP_EXTRACTION__ENABLED` | extraction | `true` |
| `MEMCP_EXTRACTION__MAX_CONTENT_CHARS` | extraction | `1500` |
| `MEMCP_EXTRACTION__OLLAMA_BASE_URL` | extraction | `"http://localhost:11434"` |
| `MEMCP_EXTRACTION__OLLAMA_MODEL` | extraction | `"llama3.2:3b"` |
| `MEMCP_EXTRACTION__OPENAI_API_KEY` | extraction | `null` |
| `MEMCP_EXTRACTION__OPENAI_MODEL` | extraction | `"gpt-4o-mini"` |
| `MEMCP_EXTRACTION__PROVIDER` | extraction | `"ollama"` |
| `MEMCP_GC__ENABLED` | gc | `true` |
| `MEMCP_GC__GC_INTERVAL_SECS` | gc | `3600` |
| `MEMCP_GC__HARD_PURGE_GRACE_DAYS` | gc | `30` |
| `MEMCP_GC__MIN_AGE_DAYS` | gc | `30` |
| `MEMCP_GC__MIN_MEMORY_FLOOR` | gc | `100` |
| `MEMCP_GC__SALIENCE_THRESHOLD` | gc | `0.3` |
| `MEMCP_HEALTH__BIND` | health | `"0.0.0.0"` |
| `MEMCP_HEALTH__ENABLED` | health | `true` |
| `MEMCP_HEALTH__PORT` | health | `9090` |
| `MEMCP_IDEMPOTENCY__DEDUP_WINDOW_SECS` | idempotency | `60` |
| `MEMCP_IDEMPOTENCY__KEY_TTL_SECS` | idempotency | `86400` |
| `MEMCP_IDEMPOTENCY__MAX_KEY_LENGTH` | idempotency | `256` |
| `MEMCP_IMPORT__BATCH_SIZE` | import | `100` |
| `MEMCP_IMPORT__DEFAULT_PROJECT` | import | `null` |
| `MEMCP_LOG_FILE` | root | `null` |
| `MEMCP_LOG_LEVEL` | root | `"info"` |
| `MEMCP_OBSERVABILITY__METRICS_ENABLED` | observability | `true` |
| `MEMCP_OBSERVABILITY__POOL_POLL_INTERVAL_SECS` | observability | `10` |
| `MEMCP_PROJECT__DEFAULT_PROJECT` | project | `null` |
| `MEMCP_QUERY_INTELLIGENCE__EXPANSION_ENABLED` | query_intelligence | `false` |
| `MEMCP_QUERY_INTELLIGENCE__EXPANSION_OLLAMA_MODEL` | query_intelligence | `"llama3.2:3b"` |
| `MEMCP_QUERY_INTELLIGENCE__EXPANSION_OPENAI_MODEL` | query_intelligence | `"gpt-5-mini"` |
| `MEMCP_QUERY_INTELLIGENCE__EXPANSION_PROVIDER` | query_intelligence | `"ollama"` |
| `MEMCP_QUERY_INTELLIGENCE__LATENCY_BUDGET_MS` | query_intelligence | `2000` |
| `MEMCP_QUERY_INTELLIGENCE__MULTI_QUERY_ENABLED` | query_intelligence | `true` |
| `MEMCP_QUERY_INTELLIGENCE__OLLAMA_BASE_URL` | query_intelligence | `"http://localhost:11434"` |
| `MEMCP_QUERY_INTELLIGENCE__OPENAI_API_KEY` | query_intelligence | `null` |
| `MEMCP_QUERY_INTELLIGENCE__OPENAI_BASE_URL` | query_intelligence | `"https://api.openai.com/v1"` |
| `MEMCP_QUERY_INTELLIGENCE__RERANKING_ENABLED` | query_intelligence | `false` |
| `MEMCP_QUERY_INTELLIGENCE__RERANKING_OLLAMA_MODEL` | query_intelligence | `"llama3.2:3b"` |
| `MEMCP_QUERY_INTELLIGENCE__RERANKING_OPENAI_MODEL` | query_intelligence | `"gpt-5-mini"` |
| `MEMCP_QUERY_INTELLIGENCE__RERANKING_PROVIDER` | query_intelligence | `"ollama"` |
| `MEMCP_QUERY_INTELLIGENCE__RERANK_CONTENT_CHARS` | query_intelligence | `500` |
| `MEMCP_RATE_LIMIT__ANNOTATE_RPS` | rate_limit | `50` |
| `MEMCP_RATE_LIMIT__BURST_MULTIPLIER` | rate_limit | `2` |
| `MEMCP_RATE_LIMIT__DELETE_RPS` | rate_limit | `50` |
| `MEMCP_RATE_LIMIT__DISCOVER_RPS` | rate_limit | `50` |
| `MEMCP_RATE_LIMIT__ENABLED` | rate_limit | `true` |
| `MEMCP_RATE_LIMIT__EXPORT_RPS` | rate_limit | `10` |
| `MEMCP_RATE_LIMIT__GLOBAL_RPS` | rate_limit | `200` |
| `MEMCP_RATE_LIMIT__RECALL_RPS` | rate_limit | `100` |
| `MEMCP_RATE_LIMIT__SEARCH_RPS` | rate_limit | `100` |
| `MEMCP_RATE_LIMIT__STORE_RPS` | rate_limit | `50` |
| `MEMCP_RATE_LIMIT__UPDATE_RPS` | rate_limit | `50` |
| `MEMCP_RECALL__BUMP_MULTIPLIER` | recall | `0.15` |
| `MEMCP_RECALL__MAX_MEMORIES` | recall | `3` |
| `MEMCP_RECALL__MIN_RELEVANCE` | recall | `0.7` |
| `MEMCP_RECALL__PREAMBLE_OVERRIDE` | recall | `null` |
| `MEMCP_RECALL__RELATED_CONTEXT_ENABLED` | recall | `true` |
| `MEMCP_RECALL__SESSION_BOOST_CAP` | recall | `0.15` |
| `MEMCP_RECALL__SESSION_BOOST_WEIGHT` | recall | `0.05` |
| `MEMCP_RECALL__SESSION_IDLE_SECS` | recall | `86400` |
| `MEMCP_RECALL__SESSION_TOPIC_TRACKING` | recall | `true` |
| `MEMCP_RECALL__STABILITY_CEILING` | recall | `100.0` |
| `MEMCP_RECALL__TAG_BOOST_CAP` | recall | `0.3` |
| `MEMCP_RECALL__TAG_BOOST_WEIGHT` | recall | `0.1` |
| `MEMCP_RECALL__TRUNCATION_CHARS` | recall | `200` |
| `MEMCP_REDACTION__ENTROPY_THRESHOLD` | redaction | `3.5` |
| `MEMCP_REDACTION__PII_ENABLED` | redaction | `false` |
| `MEMCP_REDACTION__SECRETS_ENABLED` | redaction | `true` |
| `MEMCP_RESOURCE_CAPS__MAX_DB_CONNECTIONS` | resource_caps | `10` |
| `MEMCP_RESOURCE_CAPS__MAX_EMBEDDING_BATCH_SIZE` | resource_caps | `64` |
| `MEMCP_RESOURCE_CAPS__MAX_MEMORIES` | resource_caps | `null` |
| `MEMCP_RESOURCE_CAPS__MAX_SEARCH_RESULTS` | resource_caps | `100` |
| `MEMCP_RESOURCE_LIMITS__AUTO_GC` | resource_limits | `false` |
| `MEMCP_RESOURCE_LIMITS__AUTO_GC_COOLDOWN_MINS` | resource_limits | `15` |
| `MEMCP_RESOURCE_LIMITS__HARD_CAP_PERCENT` | resource_limits | `110` |
| `MEMCP_RESOURCE_LIMITS__WARN_PERCENT` | resource_limits | `80` |
| `MEMCP_SALIENCE__DEBUG_SCORING` | salience | `false` |
| `MEMCP_SALIENCE__RECENCY_LAMBDA` | salience | `0.01` |
| `MEMCP_SALIENCE__W_ACCESS` | salience | `0.15` |
| `MEMCP_SALIENCE__W_RECENCY` | salience | `0.25` |
| `MEMCP_SALIENCE__W_REINFORCE` | salience | `0.15` |
| `MEMCP_SALIENCE__W_SEMANTIC` | salience | `0.45` |
| `MEMCP_STATUS_LINE__FORMAT` | status_line | `"ingest"` |
| `MEMCP_STORE__SYNC_TIMEOUT_SECS` | store | `5` |
| `MEMCP_SUMMARIZATION__ENABLED` | summarization | `false` |
| `MEMCP_SUMMARIZATION__MAX_INPUT_CHARS` | summarization | `4000` |
| `MEMCP_SUMMARIZATION__OLLAMA_BASE_URL` | summarization | `"http://localhost:11434"` |
| `MEMCP_SUMMARIZATION__OLLAMA_MODEL` | summarization | `"llama3.2:3b"` |
| `MEMCP_SUMMARIZATION__OPENAI_API_KEY` | summarization | `null` |
| `MEMCP_SUMMARIZATION__OPENAI_BASE_URL` | summarization | `"https://api.openai.com/v1"` |
| `MEMCP_SUMMARIZATION__OPENAI_MODEL` | summarization | `"gpt-4o-mini"` |
| `MEMCP_SUMMARIZATION__PROVIDER` | summarization | `"ollama"` |
| `MEMCP_TEMPORAL__LLM_ENABLED` | temporal | `false` |
| `MEMCP_TEMPORAL__OLLAMA_MODEL` | temporal | `"llama3.2:3b"` |
| `MEMCP_TEMPORAL__OPENAI_API_KEY` | temporal | `null` |
| `MEMCP_TEMPORAL__OPENAI_BASE_URL` | temporal | `null` |
| `MEMCP_TEMPORAL__OPENAI_MODEL` | temporal | `"gpt-4o-mini"` |
| `MEMCP_TEMPORAL__PROVIDER` | temporal | `"ollama"` |
| `MEMCP_USER__BIRTH_YEAR` | user | `null` |
