# Phase 24: Deferred Items

## Pre-existing Test Failures

### test_tool_discovery count mismatch
- **File:** crates/memcp-core/tests/integration_test.rs:340
- **Issue:** Test expects exactly 13 MCP tools but 16 are found. Prior phases added tools without updating the assertion count.
- **Not caused by Phase 24** -- no MCP tools were added in this phase.
- **Fix:** Update the assertion to expect the correct tool count (16).
