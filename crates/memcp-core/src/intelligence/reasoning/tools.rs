//! Memory tool definitions + dispatcher (REAS-06).
//!
//! 6 tools: search_memories, create_memory, update_memory, delete_memory,
//! annotate_memory, select_final_memories.
//!
//! Per CLAUDE.md agent-first rule: all dispatch goes through MemoryStore trait
//! (NO direct Postgres writes). Storage errors wrap as ToolResult { is_error: true }
//! with a structured JSON body `{"error": "...", "code": "..."}` so the model self-repairs.
//!
//! Reviews notes:
//!   - HIGH #3: ALL knowledge_tier enums use Phase 24 canonical values [raw, imported, explicit, derived, pattern].
//!   - HIGH #5: delete_memory exposes optional `force_if_source: boolean` to bypass D-06 with a warning.
//!   - MEDIUM #6: dispatch_tool validates call.arguments against the tool's JSON Schema BEFORE serde deserialize.
//!   - MEDIUM #7: ALL error payloads are JSON `{"error":..., "code":...}`. No plain-text errors.
//!
//! **Deviation note (search_memories, Rule 3):** the plan referenced a
//! `recall::recall(store, query, limit, tier)` free function that does not
//! exist — the real `RecallEngine::recall` takes an embedding (not a query
//! string) and is bound to `PostgresMemoryStore` concretely. Because
//! `MemoryStore` is the trait boundary for this dispatcher (agent-first rule),
//! `search_memories` delegates to `MemoryStore::list` with a ListFilter and
//! applies an in-memory substring match for the MVP. Hybrid/semantic search
//! via the dispatcher is tracked as a follow-up (Plan 25-06 will exercise this
//! path at the runner level; Phase 27 agentic retrieval will route to
//! hybrid_search through the retrieval specialist's tool palette).

use serde::Deserialize;
use serde_json::{json, Value};

use super::{AgentCallerContext, ReasoningError, Tool, ToolCall, ToolResult};
use crate::storage::store::{CreateMemory, ListFilter, UpdateMemory};

/// Return the full 6-tool palette. Caller passes this to generate() and dispatch_tool().
pub fn memory_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "search_memories".into(),
            description: "Semantic + hybrid search over memcp. Returns memory IDs and 200-char content snippets. Call before create_memory to check for existing entries.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural-language query"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10},
                    "tier": {"type": "string", "enum": ["raw","imported","explicit","derived","pattern","all"]}
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "create_memory".into(),
            description: "Store a new memory. knowledge_tier 'derived' requires non-empty source_ids referencing evidence memories (Phase 24 invariant).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "minLength": 1},
                    "knowledge_tier": {"type": "string", "enum": ["raw","imported","explicit","derived","pattern"]},
                    "source_ids": {"type": "array", "items": {"type": "string"}},
                    "tags": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["content", "knowledge_tier"]
            }),
        },
        Tool {
            name: "update_memory".into(),
            description: "Update a memory's content or tags. knowledge_tier and source_ids are immutable post-creation.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "content": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["id"]
            }),
        },
        Tool {
            name: "delete_memory".into(),
            description: "Delete a memory. Refuses if the memory is a source_id of any live derived memory (Phase 24 D-06) UNLESS force_if_source=true. WARNING: force_if_source=true may leave orphaned derived memories — use only when you intend to delete the source deliberately and will tombstone/repair the derived memories yourself.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "force_if_source": {"type": "boolean", "default": false, "description": "Bypass D-06 cascade guard. Emits warning in the ToolResult. Default false."}
                },
                "required": ["id"]
            }),
        },
        Tool {
            name: "annotate_memory".into(),
            description: "Append a text annotation to a memory's metadata. Parent content is never mutated; updated_at is bumped.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "annotation": {"type": "string", "minLength": 1}
                },
                "required": ["id", "annotation"]
            }),
        },
        Tool {
            name: "select_final_memories".into(),
            description: "Mark the final answer set of memory IDs for this reasoning run. Emits REAS-10 stability boost (x1.3) on these memories after the loop terminates.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1}
                },
                "required": ["ids"]
            }),
        },
    ]
}

/// Validate every tool's parameters schema is well-formed JSON Schema.
/// Call at server startup (RESEARCH Pitfall 7).
pub fn validate_tool_schemas(tools: &[Tool]) -> Result<(), ReasoningError> {
    for t in tools {
        jsonschema::validator_for(&t.parameters)
            .map_err(|e| ReasoningError::BadToolSchema(t.name.clone(), e.to_string()))?;
    }
    Ok(())
}

// ─── Dispatch ─────────────────────────────────────────────────────────

/// Error-wrapping helper: build a ToolResult { is_error: true } with a **structured JSON** body.
/// MEDIUM #7: no plain text allowed here.
fn err_result(call: &ToolCall, code: &str, msg: impl std::fmt::Display) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        content: json!({"error": msg.to_string(), "code": code}).to_string(),
        is_error: true,
    }
}

fn ok_result(call: &ToolCall, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        content,
        is_error: false,
    }
}

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    tier: Option<String>,
}
fn default_limit() -> usize {
    10
}

#[derive(Deserialize)]
struct CreateArgs {
    content: String,
    knowledge_tier: String,
    #[serde(default)]
    source_ids: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct UpdateArgs {
    id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DeleteArgs {
    id: String,
    #[serde(default)]
    force_if_source: bool,
}

#[derive(Deserialize)]
struct AnnotateArgs {
    id: String,
    annotation: String,
}

#[derive(Deserialize)]
struct SelectArgs {
    ids: Vec<String>,
}

/// Build a CreateMemory with the minimum set of fields populated by the tool dispatcher.
/// `CreateMemory` does not `impl Default` (audience/actor_type have non-None serde defaults
/// that are hard-coded here to keep dispatch predictable).
fn build_create_memory(args: CreateArgs) -> CreateMemory {
    CreateMemory {
        content: args.content,
        type_hint: "fact".into(),
        source: "reasoning-agent".into(),
        tags: args.tags,
        created_at: None,
        actor: None,
        actor_type: "agent".into(),
        audience: "global".into(),
        idempotency_key: None,
        event_time: None,
        event_time_precision: None,
        project: None,
        trust_level: None,
        session_id: None,
        agent_role: None,
        write_path: Some("reasoning_agent".into()),
        knowledge_tier: Some(args.knowledge_tier),
        source_ids: args.source_ids,
        reply_to_id: None,
    }
}

/// Dispatch one tool call against the caller context. Never panics;
/// always returns a ToolResult (error wrapped via is_error with JSON body).
///
/// MEDIUM #6 flow:
///   1. Resolve the Tool definition (unknown name → unknown_tool error).
///   2. Validate call.arguments against tool.parameters (jsonschema) → schema_validation error on fail.
///   3. Typed serde_json::from_value deserialize → bad_args error on fail.
///   4. Dispatch to the concrete handler; storage errors → storage_error.
pub async fn dispatch_tool(call: &ToolCall, ctx: &AgentCallerContext) -> ToolResult {
    // Step 1: find tool.
    let tools = memory_tools();
    let Some(tool) = tools.iter().find(|t| t.name == call.name) else {
        return err_result(call, "unknown_tool", format!("unknown tool: {}", call.name));
    };

    // Step 2: per-call JSON Schema validation (MEDIUM #6).
    match jsonschema::validator_for(&tool.parameters) {
        Ok(validator) => {
            if let Err(e) = validator.validate(&call.arguments) {
                return err_result(
                    call,
                    "schema_validation",
                    format!(
                        "arguments failed JSON Schema validation for {}: {}",
                        call.name, e
                    ),
                );
            }
        }
        Err(e) => {
            // Should be caught at startup by validate_tool_schemas, but defensive:
            return err_result(
                call,
                "bad_tool_schema",
                format!("tool {} has an invalid schema: {}", call.name, e),
            );
        }
    }

    // Step 3 + 4 dispatch (typed deserialize inside each arm).
    match call.name.as_str() {
        "search_memories" => {
            let args: SearchArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            // Tier filter: None or "all" = no filter.
            let tier_filter: Option<&str> = match args.tier.as_deref() {
                Some("all") | None => None,
                Some(other) => Some(other),
            };
            // Deviation Rule 3: MemoryStore trait has no semantic search entry point;
            // dispatch via list() + in-memory substring filter. Plan 27 will extend the
            // retrieval specialist to use hybrid_search through the concrete store.
            let filter = ListFilter {
                // Over-fetch so the substring filter still has hits after truncation.
                limit: (args.limit * 4).clamp(10, 200) as i64,
                ..Default::default()
            };
            let page = match ctx.store.list(filter).await {
                Ok(p) => p,
                Err(e) => return err_result(call, "storage_error", format!("search failed: {e}")),
            };
            let q_lower = args.query.to_lowercase();
            let mut payload: Vec<Value> = Vec::new();
            let mut hit_ids: Vec<String> = Vec::new();
            for m in &page.memories {
                if payload.len() >= args.limit {
                    break;
                }
                if let Some(tf) = tier_filter {
                    if m.knowledge_tier != tf {
                        continue;
                    }
                }
                if !m.content.to_lowercase().contains(&q_lower) {
                    continue;
                }
                let snippet: String = m.content.chars().take(200).collect();
                payload.push(json!({
                    "id": m.id,
                    "content_snippet": snippet,
                    "tier": m.knowledge_tier,
                }));
                hit_ids.push(m.id.clone());
            }
            if let Ok(mut rbd) = ctx.read_but_discarded.lock() {
                for id in &hit_ids {
                    rbd.insert(id.clone());
                }
            }
            ok_result(
                call,
                serde_json::to_string(&payload).unwrap_or_else(|_| "[]".into()),
            )
        }
        "create_memory" => {
            let args: CreateArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            let source_ids_clone = args.source_ids.clone();
            let input = build_create_memory(args);
            match ctx.store.store_with_outcome(input).await {
                Ok(outcome) => {
                    // StoreOutcome variants: Created(Memory) | Deduplicated(Memory).
                    let id = outcome.memory().id.clone();
                    if let Some(src_ids) = source_ids_clone {
                        if let Ok(mut fs) = ctx.final_selection.lock() {
                            for s in &src_ids {
                                fs.insert(s.clone());
                            }
                        }
                    }
                    ok_result(call, json!({"id": id}).to_string())
                }
                Err(e) => err_result(call, "storage_error", format!("create_memory: {e}")),
            }
        }
        "update_memory" => {
            let args: UpdateArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            let id_echo = args.id.clone();
            let update = UpdateMemory {
                content: args.content,
                tags: args.tags,
                ..Default::default()
            };
            match ctx.store.update(&args.id, update).await {
                Ok(_) => ok_result(call, json!({"updated": id_echo}).to_string()),
                Err(e) => err_result(call, "storage_error", format!("update_memory: {e}")),
            }
        }
        "delete_memory" => {
            let args: DeleteArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            // Phase 24 D-06 guard, with HIGH #5 escape hatch.
            let is_source = match ctx.store.is_source_of_any_derived(&args.id).await {
                Ok(b) => b,
                Err(e) => {
                    return err_result(call, "storage_error", format!("delete guard check: {e}"))
                }
            };
            if is_source && !args.force_if_source {
                return err_result(
                    call,
                    "cascade_delete_forbidden",
                    format!(
                        "cannot delete: memory {} is a source of one or more derived memories \
                         (Phase 24 D-06). Pass force_if_source=true to bypass; you are \
                         responsible for cleaning up orphaned derived memories.",
                        args.id
                    ),
                );
            }
            let warning_opt: Option<String> = if is_source && args.force_if_source {
                tracing::warn!(
                    memory_id = %args.id,
                    run_id = %ctx.run_id,
                    event = "delete_memory_force_if_source_bypass",
                    "delete_memory force_if_source=true bypassing D-06 guard — may leave orphaned derived memories"
                );
                Some(format!(
                    "force_if_source=true bypassed D-06 guard for {}; derived memories \
                     referencing this id are now orphaned — tombstone or repair them.",
                    args.id
                ))
            } else {
                None
            };
            match ctx.store.delete(&args.id).await {
                Ok(()) => {
                    let mut body = json!({"deleted": args.id});
                    if let Some(w) = warning_opt {
                        body["warning"] = json!(w);
                    }
                    ok_result(call, body.to_string())
                }
                Err(e) => err_result(call, "storage_error", format!("delete_memory: {e}")),
            }
        }
        "annotate_memory" => {
            let args: AnnotateArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            match ctx.store.add_annotation(&args.id, &args.annotation).await {
                Ok(()) => ok_result(call, json!({"annotated": args.id}).to_string()),
                Err(e) => err_result(call, "storage_error", format!("annotate_memory: {e}")),
            }
        }
        "select_final_memories" => {
            let args: SelectArgs = match serde_json::from_value(call.arguments.clone()) {
                Ok(a) => a,
                Err(e) => return err_result(call, "bad_args", format!("invalid args: {e}")),
            };
            if let Ok(mut fs) = ctx.final_selection.lock() {
                for id in &args.ids {
                    fs.insert(id.clone());
                }
            }
            if let Ok(mut rbd) = ctx.read_but_discarded.lock() {
                for id in &args.ids {
                    rbd.remove(id);
                }
            }
            ok_result(call, json!({"selected": args.ids.len()}).to_string())
        }
        // Unreachable — Step 1 already handled unknown names, but defensive:
        other => err_result(call, "unknown_tool", format!("unknown tool: {other}")),
    }
}

// ─── Static schema validation tests ───────────────────────────────────

#[cfg(test)]
mod tool_schema_tests {
    use super::*;

    #[test]
    fn tool_palette_has_six_tools() {
        let tools = memory_tools();
        assert_eq!(tools.len(), 6);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for expected in &[
            "search_memories",
            "create_memory",
            "update_memory",
            "delete_memory",
            "annotate_memory",
            "select_final_memories",
        ] {
            assert!(names.contains(expected), "missing tool {expected}");
        }
    }

    #[test]
    fn tool_schemas_are_valid_json_schema() {
        validate_tool_schemas(&memory_tools())
            .expect("all 6 schemas must be valid JSON Schema draft-07");
    }

    #[test]
    fn knowledge_tier_enum_uses_canonical_phase24_values() {
        // HIGH #3: guard against drift back to episodic/semantic.
        let tools = memory_tools();
        let create = tools.iter().find(|t| t.name == "create_memory").unwrap();
        let s = serde_json::to_string(&create.parameters).unwrap();
        for canonical in &["raw", "imported", "explicit", "derived", "pattern"] {
            assert!(
                s.contains(&format!("\"{canonical}\"")),
                "create_memory.knowledge_tier enum must contain canonical {canonical}"
            );
        }
        for forbidden in &["episodic", "semantic"] {
            assert!(
                !s.contains(&format!("\"{forbidden}\"")),
                "create_memory.knowledge_tier must NOT contain pre-Phase-24 value {forbidden}"
            );
        }
    }

    #[test]
    fn delete_memory_schema_exposes_force_if_source() {
        // HIGH #5: force_if_source exists as an optional boolean with default false.
        let tools = memory_tools();
        let del = tools.iter().find(|t| t.name == "delete_memory").unwrap();
        let s = serde_json::to_string(&del.parameters).unwrap();
        assert!(
            s.contains("force_if_source"),
            "delete_memory must expose force_if_source"
        );
        assert!(
            s.contains("\"boolean\""),
            "force_if_source must be boolean"
        );
    }
}
