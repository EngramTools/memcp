# Deferred Items - Phase 20

## Pre-existing Test Failure

- `test_tool_discovery` in `integration_test.rs` expects 12 tools but finds 13. Tool count is stale (a new tool was added without updating the assertion). Not caused by phase 20 changes.
