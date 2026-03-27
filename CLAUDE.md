# memcp

Rust MCP memory server. Build: `cargo build`, test: `cargo test`.

## Design Rule: Agent-First Perspective

When making decisions about memory features, search behavior, or how data surfaces to consumers:

- Reason from the perspective of an AI agent USING the memory system
- Think: "If I'm the agent storing/recalling memories, what behavior helps me most?"
- Agents access memcp through two paths: direct MCP tool calls AND code execution sandbox (JS code calling `search_memory()` as a function)
- Both paths pass parameters generically — memcp features must work identically regardless of access path
- Tool schema descriptions are the primary discovery mechanism — make them good enough that models use params correctly without system prompt hints
