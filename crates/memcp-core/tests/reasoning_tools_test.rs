//! Phase 25 Plan 05 — Wave 0 scaffold flipped GREEN.
//! Smoke tests for the 6-tool palette.

use memcp::intelligence::reasoning::{memory_tools, validate_tool_schemas};

#[test]
fn memory_tools_list_has_six_entries() {
    let tools = memory_tools();
    assert_eq!(tools.len(), 6);
    validate_tool_schemas(&tools).expect("schemas valid");
}
