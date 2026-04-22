// Phase 25 Wave 0 scaffolds — RED until plan 05 lands.
// Do NOT remove #[ignore] until plan 05 is implementing these tests.

#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 05"]
async fn test_derived_requires_source_ids_is_error() {
    unimplemented!("plan 05 delivers this test");
}

#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 05"]
async fn test_delete_blocked_on_derived() {
    unimplemented!("plan 05 delivers this test");
}

// Reviews HIGH #5: delete with force_if_source=true bypasses the D-06 guard
// (operator escape hatch — agent cannot request this flag).
#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 05"]
async fn test_delete_force_if_source_bypasses_guard() {
    unimplemented!("plan 05 delivers this test");
}

// Reviews HIGH #3: create_memory rejects unknown knowledge_tier values at the
// dispatcher (defense-in-depth vs. Phase 24 D-02 whitelist).
#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 05"]
async fn test_create_memory_rejects_unknown_knowledge_tier() {
    unimplemented!("plan 05 delivers this test");
}

// Reviews MEDIUM #6: dispatcher validates tool-call arguments against each tool's
// input_schema (jsonschema 0.46) before invoking the handler.
#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 05"]
async fn test_dispatcher_validates_args_against_schema() {
    unimplemented!("plan 05 delivers this test");
}
