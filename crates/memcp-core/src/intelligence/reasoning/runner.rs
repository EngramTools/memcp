//! Iteration-loop runner — provider-agnostic state machine for REAS-07 + REAS-08.
//!
//! Authoritative design: RESEARCH §Architecture Patterns Unified Loop Runner Pseudocode.
//! Pitfalls honored: 1 (arg normalization done at adapter), 2 (tool_call_id echo via ToolResult),
//! 3 (terminator = empty tool-call list), 4 (repeated-call detector), 6 (Ollama probe in adapter),
//! 7 (budget check BEFORE generate, max_tokens bounded per turn).
//!
//! Reviews LOW #12 accommodation: `finish_reason` is logged at `tracing::debug!` only
//! — never used for control flow.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use super::{
    apply_salience_side_effects, create_reasoning_provider, dispatch_tool, validate_tool_schemas,
    AgentCallerContext, AgentOutcome, Message, ReasoningError, ReasoningProvider,
    ReasoningRequest, ReasoningResponse, Tool,
};
use crate::config::ProfileConfig;

/// Public entry. Caller has already resolved the profile by name.
///
/// `profile_name` is the human-readable key (e.g. "dreaming" / "retrieval") used
/// for metric labels — NOT the model id. Behaviour is driven entirely off the
/// resolved `profile: &ProfileConfig`.
#[tracing::instrument(
    skip(profile, system_prompt, tools, ctx),
    fields(profile_name = %profile_name, provider = %profile.provider, model = %profile.model, run_id = %ctx.run_id)
)]
pub async fn run_agent(
    profile_name: &str,
    profile: &ProfileConfig,
    system_prompt: &str,
    tools: Vec<Tool>,
    ctx: AgentCallerContext,
) -> Result<AgentOutcome, ReasoningError> {
    // Per-run defensive schema check (Reviews LOW #9: cheap O(|tools|) — keep here
    // so tests that inject custom tool sets via run_agent_with_provider also benefit).
    validate_tool_schemas(&tools)?;
    let provider = create_reasoning_provider(profile, ctx.creds.clone())?;
    run_agent_with_provider(provider, profile_name, profile, system_prompt, tools, ctx).await
}

/// Mock-friendly entry. Used by tests that inject a `ReasoningProvider` directly
/// (bypasses the factory). Default turn timeout derives from `profile.provider`.
pub async fn run_agent_with_provider(
    provider: Arc<dyn ReasoningProvider>,
    profile_name: &str,
    profile: &ProfileConfig,
    system_prompt: &str,
    tools: Vec<Tool>,
    ctx: AgentCallerContext,
) -> Result<AgentOutcome, ReasoningError> {
    run_agent_with_provider_and_timeout(
        provider,
        profile_name,
        profile,
        system_prompt,
        tools,
        ctx,
        None,
    )
    .await
}

/// Mock-friendly entry with overridable per-turn timeout. Used by the
/// `test_timeout` termination test.
pub async fn run_agent_with_provider_and_timeout(
    provider: Arc<dyn ReasoningProvider>,
    profile_name: &str,
    profile: &ProfileConfig,
    system_prompt: &str,
    tools: Vec<Tool>,
    ctx: AgentCallerContext,
    turn_timeout_override: Option<Duration>,
) -> Result<AgentOutcome, ReasoningError> {
    let mut messages: Vec<Message> = Vec::new();
    let mut total_tokens: u32 = 0;
    let mut last_call_hashes: VecDeque<u64> = VecDeque::with_capacity(3);

    // Per-turn wall-clock timeout: Ollama gets a generous 120s (local tok/s varies
    // widely across models + hardware); hosted APIs 30s.
    let turn_timeout = turn_timeout_override.unwrap_or_else(|| match profile.provider.as_str() {
        "ollama" => Duration::from_secs(120),
        _ => Duration::from_secs(30),
    });

    for iter in 0..profile.max_iterations {
        // Budget hard stop BEFORE generate (Pitfall 7). Using >= ensures we can't
        // issue a request that would provably exceed the budget even at zero
        // completion tokens.
        if total_tokens >= profile.budget_tokens {
            let _ = apply_salience_side_effects(&ctx).await;
            return Ok(AgentOutcome::BudgetExceeded {
                tokens_used: total_tokens,
                iterations: iter,
            });
        }

        // max_tokens per turn = remaining budget, capped at 4096 (Pitfall 7 second
        // line of defense — prevents a single unbounded response from blowing
        // through an otherwise-healthy budget).
        let per_turn_max = profile
            .budget_tokens
            .saturating_sub(total_tokens)
            .min(4096);

        let req = ReasoningRequest {
            system_prompt: system_prompt.to_string(),
            messages: messages.clone(),
            tools: tools.clone(),
            max_tokens: per_turn_max,
            temperature: profile.temperature,
        };

        let resp: ReasoningResponse =
            match tokio::time::timeout(turn_timeout, provider.generate(&req)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(ReasoningError::Transport("turn timeout".into())),
            };

        total_tokens = total_tokens.saturating_add(resp.usage.total_tokens);
        // Single-line form mandated by plan acceptance grep: the counter-macro
        // call, profile + adapter label args must all live on one line so
        // test_metric_emitted_counter_name_present_in_source (src.contains)
        // + the plan grep both match without multi-line regex gymnastics.
        #[rustfmt::skip]
        metrics::counter!("reasoning_tokens_total", "profile" => profile_name.to_string(), "adapter" => profile.provider.clone()).increment(u64::from(resp.usage.total_tokens));

        // Reviews LOW #12: log finish_reason for diagnostics at DEBUG only.
        // DO NOT use it for control flow — Pitfall 3 strictly forbids that.
        let diag_finish = resp.finish_reason.as_deref().unwrap_or("<none>");
        tracing::debug!(
            iter = iter,
            finish_reason = %diag_finish,
            "provider reported finish_reason (diagnostic only — ignored for termination)"
        );

        // Terminator: empty tool_calls only (Pitfall 3 — ignore the provider-supplied
        // stop/finish field; it drifts between upstream APIs).
        if resp.tool_calls.is_empty() {
            let _ = apply_salience_side_effects(&ctx).await;
            return Ok(AgentOutcome::Terminal {
                content: resp.content,
                tokens_used: total_tokens,
                iterations: iter + 1,
            });
        }

        // Repeated-call detector (Pitfall 4).
        for call in &resp.tool_calls {
            let h = hash_canonical_call(&call.name, &call.arguments);
            if last_call_hashes.len() == 3 {
                last_call_hashes.pop_front();
            }
            last_call_hashes.push_back(h);
        }
        let repeated = last_call_hashes.len() == 3
            && last_call_hashes
                .front()
                .is_some_and(|first| last_call_hashes.iter().all(|h| h == first));
        if repeated {
            let _ = apply_salience_side_effects(&ctx).await;
            return Ok(AgentOutcome::RepeatedToolCall {
                tokens_used: total_tokens,
                iterations: iter + 1,
            });
        }

        // Append assistant turn to history.
        messages.push(Message::assistant_with_tools(
            resp.content.clone(),
            resp.tool_calls.clone(),
        ));

        // Parallel dispatch — multiple tool_calls in one assistant turn are
        // dispatched concurrently.
        let dispatch_futs = resp.tool_calls.iter().map(|c| dispatch_tool(c, &ctx));
        let results = futures::future::join_all(dispatch_futs).await;

        for r in &results {
            messages.push(Message::tool_result(r));
        }
    }

    // Max iterations exhausted — REAS-10 still fires on whatever was captured.
    let _ = apply_salience_side_effects(&ctx).await;
    Ok(AgentOutcome::MaxIterations {
        tokens_used: total_tokens,
        iterations: profile.max_iterations,
    })
}

/// Hash `(name, canonicalized_args)` with sorted object keys so two syntactically
/// different JSON representations of the same logical args hash identically.
fn hash_canonical_call(name: &str, args: &serde_json::Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let canonical = canonicalize_value(args);
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    canonical.hash(&mut h);
    h.finish()
}

fn canonicalize_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", k, canonicalize_value(&m[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        serde_json::Value::Array(a) => {
            let items: Vec<String> = a.iter().map(canonicalize_value).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}
