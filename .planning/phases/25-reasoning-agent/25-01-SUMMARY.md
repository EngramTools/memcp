---
phase: 25-reasoning-agent
plan: 01
subsystem: reasoning
tags: [rust, async-trait, config, reasoning, trait, factory, byok]
requirements: [REAS-01, REAS-09]
dependency_graph:
  requires:
    - Phase 25 Plan 00 (27 RED scaffolds incl. reasoning_trait_test, jsonschema + wiremock deps)
    - existing SummarizationProvider trait pattern (81-line mod.rs) as template
  provides:
    - "intelligence::reasoning::ReasoningProvider (async trait, object-safe)"
    - "unified types: Tool, ToolCall, ToolResult, Message, TokenUsage, ReasoningRequest, ReasoningResponse, AgentOutcome, AgentCallerContext"
    - "ProviderCredentials { api_key, base_url } + from_env / from_headers / require_api_key"
    - "create_reasoning_provider factory with kimi/openai/ollama match arms"
    - "config::ReasoningConfig + ProfileConfig with seed dreaming + retrieval profiles"
    - "config.reasoning.resolve(name) with default_profile fallback"
    - "From<ReasoningError> for MemcpError bridge"
  affects:
    - "Plan 25-02 (Kimi adapter — replaces kimi.rs stub)"
    - "Plan 25-03 (OpenAI adapter — replaces openai.rs stub)"
    - "Plan 25-04 (Ollama adapter — replaces ollama.rs stub)"
    - "Plan 25-05 (tool dispatch imports Tool/ToolCall/ToolResult)"
    - "Plan 25-06 (loop runner imports ReasoningRequest/Response + AgentOutcome)"
    - "Plan 25-07 (salience hook consumes AgentCallerContext selection sets)"
    - "Plan 25-08 (BYOK middleware calls ProviderCredentials::from_headers)"
tech_stack:
  added: []
  patterns:
    - "Unified internal tool shape (D-04) — adapters translate in/out at their own boundary"
    - "Credentials passed into factory (D-09) — trait never reads env or headers"
    - "Stub adapter modules returning NotConfigured from new() — lets factory wire up before adapter bodies land"
    - "Async trait via async_trait crate; Send + Sync required; serde Serialize/Deserialize on every wire type"
key_files:
  created:
    - crates/memcp-core/src/intelligence/reasoning/mod.rs
    - crates/memcp-core/src/intelligence/reasoning/credentials.rs
    - crates/memcp-core/src/intelligence/reasoning/kimi.rs
    - crates/memcp-core/src/intelligence/reasoning/openai.rs
    - crates/memcp-core/src/intelligence/reasoning/ollama.rs
  modified:
    - crates/memcp-core/src/intelligence/mod.rs
    - crates/memcp-core/src/config.rs
    - crates/memcp-core/tests/reasoning_trait_test.rs
decisions:
  - ReasoningProvider trait keeps the 2-method shape (generate + model_name) — deliberately mirrors SummarizationProvider / QueryIntelligence so callers learn one pattern
  - ToolCall.arguments is always a parsed serde_json::Value — adapters normalize stringified JSON at translate_out boundary (RESEARCH Pitfall 1)
  - Message enum is serde-tagged on "role" with lowercase rename — OpenAI/Kimi wire format is a cheap adapter mapping, not the public shape
  - finish_reason kept on ReasoningResponse but documented "debug-only, NEVER loop terminator" (RESEARCH Pitfall 3)
  - AgentCallerContext uses std::sync::Mutex<HashSet<String>> (not tokio::Mutex) for the 3 selection sets — these are in-memory bookkeeping with no async I/O
  - base_url is NEVER populated on the BYOK path — only Pro env reads permit it, adapter defaults win on BYOK (SSRF T-25-01-01 mitigation)
  - Task 2 (config) committed BEFORE Task 1 (trait) because the trait module imports ProfileConfig — plan listed them in reverse logical order; executor reordered
metrics:
  duration_minutes: ~12
  tasks_completed: 2
  files_created: 5
  files_modified: 3
  commits: 2
  tests_added: 4
  tests_green: 4
  tests_ignored: 0
  completed_at: "2026-04-20T00:00:00Z"
---

# Phase 25 Plan 01: ReasoningProvider Trait + Config Foundation Summary

Establishes the `ReasoningProvider` async trait, 9 unified wire types, `ProviderCredentials`, a 3-arm factory, and the `[reasoning]` config section with seed `dreaming` + `retrieval` profiles — the contract Plans 02-08 extend.

## What Shipped

**Trait + unified types** (`intelligence/reasoning/mod.rs`):

```
#[async_trait]
pub trait ReasoningProvider: Send + Sync {
    async fn generate(&self, req: &ReasoningRequest) -> Result<ReasoningResponse, ReasoningError>;
    fn model_name(&self) -> &str;
}
```

Unified types land in one place: `Tool`, `ToolCall` (arguments: parsed Value), `ToolResult`, `Message` (serde-tagged on `role`), `TokenUsage`, `ReasoningRequest`, `ReasoningResponse`, `AgentOutcome` (4 variants — Terminal, BudgetExceeded, MaxIterations, RepeatedToolCall), and `AgentCallerContext` (store + creds + run_id + 3 HashSet<String> selection trackers).

**`ProviderCredentials`** (`credentials.rs`):
- `from_env(provider)` reads `MEMCP_REASONING__<PROVIDER>_API_KEY` + `_BASE_URL`
- `from_headers(&HeaderMap)` parses `x-reasoning-api-key` (BYOK); `base_url` hard-coded to `None` on this path — adapter defaults always win (SSRF T-25-01-01)
- `require_api_key(provider)` returns a `NotConfigured` naming both the Pro env var and the BYOK header so the caller knows both remedies

**Factory** (`create_reasoning_provider`):

```
match profile.provider.as_str() {
    "kimi"   => kimi::KimiReasoningProvider::new(profile, creds)     .map(Arc::new as _),
    "openai" => openai::OpenAIReasoningProvider::new(profile, creds) .map(Arc::new as _),
    "ollama" => ollama::OllamaReasoningProvider::new(profile, creds) .map(Arc::new as _),
    other    => Err(NotConfigured(format!("unknown provider: {other}"))),
}
```

The 3 adapter modules each ship a `new()` that returns `NotConfigured("<provider> adapter — plan NN")`. Plans 02/03/04 replace the stub bodies with real HTTP clients.

**Config** (`config.rs`):
- `ReasoningConfig { default_profile, profiles: HashMap<String, ProfileConfig> }` with `#[serde(default)]` on the `profiles` map and `#[serde(default = "default_reasoning_default_profile")]` on `default_profile`
- `ProfileConfig { provider, model, max_iterations, budget_tokens, temperature, api_key, base_url }` with per-field defaults (12 iter / 8k budget / 0.2 temp)
- Seed `dreaming`: kimi + kimi-k2.5 + 12 iter + 32k budget + 0.3 temp
- Seed `retrieval`: kimi + kimi-latest + 6 iter + 8k budget + 0.2 temp
- `default_profile = "retrieval"` — cheap path wins when caller omits
- `resolve(name)` returns `profiles[default_profile]` for empty name, `profiles[name]` otherwise, `None` on unknown
- Wired into top-level `Config` struct with `#[serde(default)] pub reasoning: ReasoningConfig`

**From<ReasoningError> for MemcpError** — routes agent errors through the existing `Internal` variant (sanitized Display impl keeps adapter strings safe).

## Key Decisions Made

1. **Plan-ordered commits reversed** — the plan listed Task 1 (trait) before Task 2 (config), but `intelligence::reasoning::create_reasoning_provider` imports `ProfileConfig`, so config must exist first for Task 1 to build. Committed config first (b42758d), then trait (f09c44f). Documented as a Rule 3 blocker fix — not a design change.
2. **Stub adapter modules ship now, not later** — plan called for stubs to "let workspace build". Kept each stub to ~40 lines with matching `new()` + `impl ReasoningProvider` so plans 02-04 diff cleanly rather than re-creating the module.
3. **`ToolCall.arguments: serde_json::Value`** — not `String`. Forces adapters to parse at their translate_out boundary; callers (plan 05 dispatcher) get a typed value and never have to re-parse. Matches RESEARCH Pitfall 1.
4. **`finish_reason` kept on `ReasoningResponse`** — documented as debug-only with inline comment pointing at RESEARCH Pitfall 3. Dropping it entirely would have cost log fidelity with no safety gain; the guardrail lives at the doc string.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocker] Test import used `memcp_core` instead of `memcp`**
- **Found during:** Task 1 (running `cargo test -p memcp-core --test reasoning_trait_test`)
- **Issue:** Plan specified `use memcp_core::intelligence::reasoning::...` but the lib crate's `name` is `memcp` (package name is `memcp-core` but `[lib] name = "memcp"`). Existing tests (`store_test.rs`, `stress_test.rs`, `import_test.rs`) all use `use memcp::...`.
- **Fix:** Changed the test import prefix to `memcp::`. Test flipped RED → GREEN immediately.
- **Files modified:** `crates/memcp-core/tests/reasoning_trait_test.rs`
- **Commit:** f09c44f

**2. [Rule 3 - Blocker] Task-order reversed to land config before trait**
- **Found during:** Pre-Task 1 planning
- **Issue:** `intelligence::reasoning::mod.rs` has `use crate::config::ProfileConfig`. Running Task 1's verify step (`cargo build -p memcp-core`) with Task 2 unfinished would fail.
- **Fix:** Committed Task 2 (ReasoningConfig) first as `b42758d`, then Task 1 (trait) as `f09c44f`. Build green after each commit. Acceptance criteria for both tasks still met.
- **Files modified:** n/a (ordering only)
- **Commit:** b42758d, f09c44f

## Verification Results

| Check | Result |
|-|-|
| `cargo build -p memcp-core` | exit 0 (4m31s clean) |
| `cargo test --no-run -p memcp-core` | exit 0 (all test bins compile) |
| `cargo test -p memcp-core --test reasoning_trait_test` | 1 passed, 0 failed, 0 ignored |
| `cargo test -p memcp-core --lib reasoning_config` | 2 passed (default_has_two_seed_profiles, resolve_falls_back_to_default_profile) |
| `cargo test -p memcp-core --lib test_config_has_reasoning_field` | 1 passed |
| `grep -c 'pub mod reasoning' intelligence/mod.rs` | 1 |
| `grep -c 'pub trait ReasoningProvider' reasoning/mod.rs` | 1 |
| `grep -c 'pub struct ReasoningRequest' reasoning/mod.rs` | 1 |
| `grep -c 'pub struct ProviderCredentials' credentials.rs` | 1 |
| `grep -c 'pub fn create_reasoning_provider' reasoning/mod.rs` | 1 |
| `grep -Ec '"kimi" =>\|"openai" =>\|"ollama" =>' reasoning/mod.rs` | 3 |
| `grep -c 'impl From<ReasoningError> for MemcpError' reasoning/mod.rs` | 1 |
| `grep -c 'pub struct ReasoningConfig' config.rs` | 1 |
| `grep -c 'pub struct ProfileConfig' config.rs` | 1 |
| `grep -c 'pub reasoning: ReasoningConfig' config.rs` | 1 |
| `grep -c 'budget_tokens: 32_000' config.rs` | 1 |
| `grep -c 'kimi-k2.5' config.rs` | 4 (seed + 2 tests + 1 comment) |

## Commits

| Hash | Message |
|-|-|
| b42758d | feat(25-01): add ReasoningConfig + ProfileConfig with seed profiles |
| f09c44f | feat(25-01): add ReasoningProvider trait + unified types + factory |

## Self-Check: PASSED

- Files exist: `crates/memcp-core/src/intelligence/reasoning/{mod,credentials,kimi,openai,ollama}.rs` — 5 files verified on disk via `git log --stat`.
- Files modified: `intelligence/mod.rs`, `config.rs`, `tests/reasoning_trait_test.rs` — all present in the 2 commits.
- Commits exist: `git log --oneline` shows b42758d and f09c44f on main.
- Trait compiles + GREEN: `cargo test -p memcp-core --test reasoning_trait_test` reports `1 passed`.
- Config tests GREEN: both `reasoning_config_*` tests + `test_config_has_reasoning_field` in `config::tests` report `ok`.
- Wave 0 scaffold flipped: `reasoning_trait_test.rs::trait_compiles` ran without `#[ignore]` and passed.
