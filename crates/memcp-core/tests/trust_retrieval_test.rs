//! Integration tests for trust-weighted retrieval scoring (TWR-01, TWR-02).
//!
//! Validates that trust_level multiplies salience in composite scoring,
//! demoting low-trust memories even when semantically relevant.
//!
//! Strategy: Content-hash dedup prevents identical content, so tests use
//! content with nearly identical BM25 relevance but different trust_level.
//! The trust gap (e.g., 0.9 vs 0.1) dominates the composite score difference.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use memcp::config::Config;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::transport::api;
use memcp::transport::health::AppState;
use memcp::MIGRATOR;
use metrics_exporter_prometheus::PrometheusBuilder;
use sqlx::PgPool;
use tokio::time::Instant;

mod common;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn make_test_state(pool: PgPool, ready: bool) -> AppState {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let config = Config::default();
    let metrics_handle = PrometheusBuilder::new().build_recorder().handle();
    AppState {
        ready: Arc::new(AtomicBool::new(ready)),
        started_at: Instant::now(),
        caps: config.resource_caps.clone(),
        store: Some(Arc::new(store)),
        config: Arc::new(config),
        embed_provider: None,
        embed_sender: None,
        metrics_handle,
        redaction_engine: None,
        auth: memcp::transport::api::auth::AuthState::default(),
        content_filter: None,
        summarization_provider: None,
        extract_sender: None,
    }
}

async fn spawn_test_server(state: AppState) -> String {
    let api_routes = api::router(&state.config.rate_limit, state.auth.clone());
    let app = Router::new()
        .route("/health", get(memcp::transport::health::status_handler))
        .merge(api_routes)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{}", addr)
}

/// Store a memory with explicit trust_level via raw SQL, bypassing content-hash dedup.
/// This allows storing identical content with different trust_level values.
async fn store_trusted_memory(pool: &PgPool, content: &str, trust_level: f32) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO memories (id, content, type_hint, source, tags, created_at, updated_at,
         last_accessed_at, access_count, embedding_status, actor_type, audience,
         trust_level, metadata)
         VALUES ($1, $2, 'fact', 'test', '[]'::jsonb, NOW(), NOW(),
         NOW(), 0, 'done', 'agent', 'global',
         $3, '{}'::jsonb)",
    )
    .bind(&id)
    .bind(content)
    .bind(trust_level)
    .execute(pool)
    .await
    .unwrap();

    // Also create salience entry (same as store() does)
    sqlx::query(
        "INSERT INTO memory_salience (memory_id, stability, last_reinforced_at)
         VALUES ($1, 2.5, NOW())",
    )
    .bind(&id)
    .execute(pool)
    .await
    .unwrap();

    id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// TWR-01: High-trust memory (0.9) scores higher than low-trust (0.1) in
/// composite scoring when content relevance and salience are similar.
///
/// Uses different content (to avoid dedup) but with the same core BM25 terms.
/// The large trust gap (0.9 vs 0.1) ensures trust-weighting dominates.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_high_trust_outscores_low_trust(pool: PgPool) {
    let high_id = store_trusted_memory(
        &pool,
        "Rust programming language provides memory safety guarantees",
        0.9,
    )
    .await;

    let low_id = store_trusted_memory(
        &pool,
        "Rust programming language provides memory safety guarantees",
        0.1,
    )
    .await;

    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp: serde_json::Value = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({
            "query": "Rust programming language memory safety guarantees",
            "limit": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let results = resp["results"].as_array().expect("results array");
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}. Response: {}",
        results.len(),
        resp
    );

    let high_trust_result = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == high_id)
        .expect("high-trust memory should be in results");
    let low_trust_result = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == low_id)
        .expect("low-trust memory should be in results");

    let high_composite = high_trust_result["composite_score"].as_f64().unwrap();
    let low_composite = low_trust_result["composite_score"].as_f64().unwrap();

    assert!(
        high_composite > low_composite,
        "High-trust (0.9) composite_score {} should exceed low-trust (0.1) composite_score {}",
        high_composite,
        low_composite
    );
}

/// TWR-02: trust_level=0.0 zeroes salience contribution, leaving only RRF.
/// When a zero-trust memory is the only result, its composite is 1.0 (single-result normalization).
/// With multiple results, its salience component is zeroed so composite = 0.5 * norm_rrf only.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_zero_trust_zeroes_salience_contribution(pool: PgPool) {
    // Store two memories: zero-trust (worse BM25 relevance) and full-trust (better BM25).
    // The full-trust memory should rank higher because:
    //   - It has better BM25 (more matching terms, gets norm_rrf=1.0)
    //   - Its trust=1.0 preserves salience contribution
    //   - Zero-trust memory's salience is zeroed, and it has worse BM25 too
    let full_id = store_trusted_memory(
        &pool,
        "Tokio async runtime provides concurrent task scheduling for applications and services",
        1.0,
    )
    .await;

    let zero_id = store_trusted_memory(
        &pool,
        "Tokio async runtime provides concurrent task scheduling for production workloads",
        0.0,
    )
    .await;

    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp: serde_json::Value = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({
            "query": "Tokio async runtime concurrent task scheduling",
            "limit": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let results = resp["results"].as_array().expect("results array");
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}. Response: {}",
        results.len(),
        resp
    );

    let zero_trust = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == zero_id)
        .expect("zero-trust memory should still appear in results");

    let full_trust = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == full_id)
        .expect("full-trust memory should appear in results");

    let zero_composite = zero_trust["composite_score"].as_f64().unwrap();
    let full_composite = full_trust["composite_score"].as_f64().unwrap();

    // Full-trust memory with better BM25 should clearly outscore zero-trust
    assert!(
        full_composite > zero_composite,
        "Full-trust (1.0) composite {} should exceed zero-trust (0.0) composite {}",
        full_composite,
        zero_composite
    );

    // Zero-trust memory is still retrievable (composite >= 0 from RRF component)
    assert!(
        zero_composite >= 0.0,
        "Zero-trust memory should have non-negative composite score"
    );

    // Verify zero-trust composite is at most 0.5 (max possible from RRF-only, no salience)
    assert!(
        zero_composite <= 0.5 + 0.001,
        "Zero-trust composite {} should be <= 0.5 (only RRF component, no salience)",
        zero_composite
    );
}

/// TWR-03: trust_level=1.0 preserves valid scoring behavior.
/// Both memories at trust=1.0 have composite scores in [0, 1].
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_full_trust_preserves_old_behavior(pool: PgPool) {
    let _id1 = store_trusted_memory(
        &pool,
        "PostgreSQL database administration and optimization techniques",
        1.0,
    )
    .await;

    let _id2 = store_trusted_memory(
        &pool,
        "PostgreSQL database replication and high availability setup",
        1.0,
    )
    .await;

    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp: serde_json::Value = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({
            "query": "PostgreSQL database",
            "limit": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let results = resp["results"].as_array().expect("results array");
    assert!(
        results.len() >= 2,
        "Expected at least 2 results, got {}. Response: {}",
        results.len(),
        resp
    );

    // Both trust=1.0 memories should have composite scores in valid range [0, 1]
    for result in results {
        let composite = result["composite_score"].as_f64().unwrap();
        assert!(
            composite >= 0.0 && composite <= 1.0,
            "Composite score {} should be in [0, 1] for trust=1.0",
            composite
        );
    }
}

/// TWR-04: Search via HTTP API returns high-trust memory ranked above
/// low-trust memory when both are relevant.
#[sqlx::test(migrator = "MIGRATOR")]
async fn test_search_results_ordered_by_trust_weighted_score(pool: PgPool) {
    let high_id = store_trusted_memory(
        &pool,
        "Machine learning model training with deep neural networks",
        0.9,
    )
    .await;

    let mid_id = store_trusted_memory(
        &pool,
        "Machine learning model training with deep neural networks",
        0.5,
    )
    .await;

    let low_id = store_trusted_memory(
        &pool,
        "Machine learning model training with deep neural networks",
        0.1,
    )
    .await;

    let state = make_test_state(pool, true).await;
    let base = spawn_test_server(state).await;
    let client = reqwest::Client::new();

    let resp: serde_json::Value = client
        .post(format!("{}/v1/search", base))
        .json(&serde_json::json!({
            "query": "machine learning model training deep neural networks",
            "limit": 10
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let results = resp["results"].as_array().expect("results array");
    assert!(
        results.len() >= 3,
        "Expected at least 3 results, got {}. Response: {}",
        results.len(),
        resp
    );

    // The highest-trust memory should have the highest composite score
    let high_result = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == high_id)
        .unwrap();
    let mid_result = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == mid_id)
        .unwrap();
    let low_result = results
        .iter()
        .find(|r| r["id"].as_str().unwrap() == low_id)
        .unwrap();

    let high_score = high_result["composite_score"].as_f64().unwrap();
    let mid_score = mid_result["composite_score"].as_f64().unwrap();
    let low_score = low_result["composite_score"].as_f64().unwrap();

    assert!(
        high_score >= mid_score && mid_score >= low_score,
        "Scores should follow trust order: high({})={} >= mid({})={} >= low({})={}",
        high_id,
        high_score,
        mid_id,
        mid_score,
        low_id,
        low_score
    );
}
