// Phase 25 Wave 0 scaffolds — RED until plan 08 lands.
// Do NOT remove #[ignore] until plan 08 is implementing these tests.

#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 08"]
async fn pro_tier_strips_caller_api_key_header() {
    unimplemented!("plan 08 delivers this test");
}

#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 08"]
async fn byok_tier_requires_headers() {
    unimplemented!("plan 08 delivers this test");
}

// Reviews HIGH #2: Ollama BYOK path does NOT require an API key header —
// self-hosted models have no auth surface.
#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 08"]
async fn byok_ollama_no_api_key_required() {
    unimplemented!("plan 08 delivers this test");
}

// Reviews HIGH #2: Pro-tier Ollama profile succeeds with no env api_key configured
// (server-side Ollama is keyless).
#[tokio::test]
#[ignore = "Wave 0 stub — pending plan 08"]
async fn pro_ollama_no_env_key_succeeds() {
    unimplemented!("plan 08 delivers this test");
}
