//! Golden dataset search quality regression tests.
//!
//! These tests use real fastembed embeddings to verify that known queries
//! return their expected top results above a minimum score threshold.
//!
//! All tests are `#[ignore]` by default — run explicitly with:
//!   cargo test --test search_quality -- --ignored --nocapture
//!
//! Feature gate: requires `local-embed` feature (enabled by default).
//!   cargo check --tests --no-default-features  → this file is skipped
#![cfg(feature = "local-embed")]

mod common;
use common::golden::load_golden_queries;

use memcp::embedding::local::LocalEmbeddingProvider;
use memcp::embedding::EmbeddingProvider;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{CreateMemory, MemoryStore};
use pgvector::Vector;
use sqlx::PgPool;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared LocalEmbeddingProvider — 87MB model, initialize once per process
// ---------------------------------------------------------------------------

/// Returns the shared LocalEmbeddingProvider, initializing it on first call.
///
/// Using OnceLock<Mutex<...>> ensures the provider is only constructed once
/// across all tests in this binary, avoiding parallel contention on the model.
static PROVIDER: OnceLock<Mutex<LocalEmbeddingProvider>> = OnceLock::new();

async fn get_provider() -> &'static Mutex<LocalEmbeddingProvider> {
    if PROVIDER.get().is_none() {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("memcp/models")
            .to_string_lossy()
            .to_string();

        let provider = LocalEmbeddingProvider::new(&cache_dir, "AllMiniLML6V2")
            .await
            .expect("Failed to initialize LocalEmbeddingProvider for golden tests");

        // OnceLock::set returns Err if already set (race in parallel tests) — that's fine
        let _ = PROVIDER.set(Mutex::new(provider));
    }
    PROVIDER.get().unwrap()
}

// ---------------------------------------------------------------------------
// DB helpers (mirroring pattern from journey_test.rs)
// ---------------------------------------------------------------------------

fn emb_str(v: &[f32]) -> String {
    format!(
        "[{}]",
        v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
    )
}

/// Store a memory and insert its real embedding, marking it complete.
async fn store_with_real_embedding(
    store: &PostgresMemoryStore,
    pool: &PgPool,
    content: &str,
    embedding: &[f32],
) -> String {
    let memory = store
        .store(CreateMemory {
            content: content.to_string(),
            type_hint: "fact".to_string(),
            source: "golden-test".to_string(),
            tags: Some(vec!["golden".to_string()]),
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            created_at: None,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: None,
            session_id: None,
            agent_role: None,
        })
        .await
        .expect("Failed to store golden memory");

    let id = memory.id.clone();

    sqlx::query(
        "INSERT INTO memory_embeddings \
         (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at) \
         VALUES ($1, $2, $3::vector, 'AllMiniLML6V2', '1', $4, true, NOW(), NOW())",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&id)
    .bind(emb_str(embedding))
    .bind(embedding.len() as i32)
    .execute(pool)
    .await
    .expect("Failed to insert real embedding");

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(&id)
        .execute(pool)
        .await
        .expect("Failed to update embedding_status");

    id
}

// ---------------------------------------------------------------------------
// Test 1: Main search quality regression test
// ---------------------------------------------------------------------------

/// Core golden dataset test: each query must return its expected content as the
/// top result, with a score above the minimum threshold.
///
/// Run with: cargo test --test search_quality test_golden_search_quality -- --ignored --nocapture
#[ignore]
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_golden_search_quality(pool: PgPool) {
    let provider_lock = get_provider().await;
    let provider = provider_lock.lock().await;

    let store = PostgresMemoryStore::from_pool(pool.clone())
        .await
        .expect("Failed to create store");

    let golden = load_golden_queries();
    assert!(
        golden.len() >= 8,
        "Golden dataset must have at least 8 entries, found {}",
        golden.len()
    );

    // Embed all seed content and store with real embeddings
    let mut stored_ids: Vec<String> = Vec::new();
    for entry in &golden {
        let embedding = provider
            .embed(&entry.seed_content)
            .await
            .expect("Failed to embed seed content");
        let id = store_with_real_embedding(&store, &pool, &entry.seed_content, &embedding).await;
        stored_ids.push(id);
    }

    // Run each query and verify results
    println!("\n--- Golden Dataset Search Quality Report ---");
    println!("{:<50} {:>8} {:>8} {:>6}", "Query (truncated)", "TopScore", "MinScore", "Pass?");
    println!("{}", "-".repeat(78));

    let mut pass_count = 0;
    let mut fail_count = 0;

    for entry in &golden {
        let query_emb = provider
            .embed(&entry.query)
            .await
            .expect("Failed to embed query");
        let query_vector = Vector::from(query_emb);

        let hits = store
            .hybrid_search(
                &entry.query,
                Some(&query_vector),
                10,
                None,
                None,
                None,
                Some(60.0),
                Some(60.0),
                Some(40.0),
                None,
                None,
                None, // project
            )
            .await
            .expect("hybrid_search failed");

        let truncated_query = if entry.query.len() > 48 {
            format!("{}...", &entry.query[..45])
        } else {
            entry.query.clone()
        };

        if hits.is_empty() {
            println!(
                "{:<50} {:>8} {:>8} {:>6}",
                truncated_query, "NO HITS", format!("{:.3}", entry.min_score), "FAIL"
            );
            eprintln!(
                "FAIL [{}]: no results returned for query '{}'",
                entry.category, entry.query
            );
            fail_count += 1;
            continue;
        }

        let top_hit = &hits[0];
        let top_score = top_hit.rrf_score as f32;
        let content_matches = top_hit
            .memory
            .content
            .contains(&entry.expected_top_content);
        let score_ok = top_score >= entry.min_score;

        let pass_str = if content_matches && score_ok {
            "PASS"
        } else {
            "FAIL"
        };

        println!(
            "{:<50} {:>8.4} {:>8.3} {:>6}",
            truncated_query, top_score, entry.min_score, pass_str
        );

        if !content_matches {
            eprintln!(
                "FAIL [{}] content mismatch:\n  Expected top to contain: '{}'\n  Got: '{}'",
                entry.category, entry.expected_top_content, top_hit.memory.content
            );
            fail_count += 1;
        } else if !score_ok {
            eprintln!(
                "FAIL [{}] score too low: expected >= {:.3}, got {:.4}\n  Query: '{}'\n  Content: '{}'",
                entry.category, entry.min_score, top_score, entry.query, top_hit.memory.content
            );
            fail_count += 1;
        } else {
            pass_count += 1;
        }
    }

    println!("{}", "-".repeat(78));
    println!("Result: {}/{} passed", pass_count, golden.len());
    println!("-------------------------------------------\n");

    assert_eq!(
        fail_count,
        0,
        "{} golden queries failed — see output above for details",
        fail_count
    );
}

// ---------------------------------------------------------------------------
// Test 2: No false negatives for unrelated queries
// ---------------------------------------------------------------------------

/// Negative test: an unrelated query (recipe for chocolate cake) must not
/// return golden memories with high scores in the top 3 results.
///
/// Run with: cargo test --test search_quality test_golden_no_false_negatives -- --ignored --nocapture
#[ignore]
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_golden_no_false_negatives(pool: PgPool) {
    let provider_lock = get_provider().await;
    let provider = provider_lock.lock().await;

    let store = PostgresMemoryStore::from_pool(pool.clone())
        .await
        .expect("Failed to create store");

    let golden = load_golden_queries();

    // Seed all golden memories with real embeddings
    for entry in &golden {
        let embedding = provider
            .embed(&entry.seed_content)
            .await
            .expect("Failed to embed seed content");
        store_with_real_embedding(&store, &pool, &entry.seed_content, &embedding).await;
    }

    // Query with something completely unrelated to tech/programming
    let unrelated_query = "recipe for chocolate cake with buttercream frosting";
    let query_emb = provider
        .embed(unrelated_query)
        .await
        .expect("Failed to embed unrelated query");
    let query_vector = Vector::from(query_emb);

    let hits = store
        .hybrid_search(
            unrelated_query,
            Some(&query_vector),
            5,
            None,
            None,
            None,
            Some(60.0),
            Some(60.0),
            Some(40.0),
            None,
            None,
            None, // project
        )
        .await
        .expect("hybrid_search failed");

    // If results exist, their RRF scores must be low (unrelated content)
    // RRF scores for unrelated content should be below 0.03 (max possible ~0.016 for single-leg)
    const HIGH_SCORE_THRESHOLD: f64 = 0.03;
    let high_score_hits: Vec<_> = hits
        .iter()
        .take(3)
        .filter(|h| h.rrf_score > HIGH_SCORE_THRESHOLD)
        .collect();

    assert!(
        high_score_hits.is_empty(),
        "Unrelated query '{}' returned {} high-score result(s) from golden memories:\n{}",
        unrelated_query,
        high_score_hits.len(),
        high_score_hits
            .iter()
            .map(|h| format!("  score={:.4}: '{}'", h.rrf_score, h.memory.content))
            .collect::<Vec<_>>()
            .join("\n")
    );

    println!(
        "PASS: unrelated query returned {} results, all with low scores (threshold={})",
        hits.len(),
        HIGH_SCORE_THRESHOLD
    );
}

// ---------------------------------------------------------------------------
// Test 3: Category isolation — preference queries return preference memories
// ---------------------------------------------------------------------------

/// Category isolation test: a preference-category query must return a
/// preference-category memory as its top result, not a fact or decision.
///
/// Run with: cargo test --test search_quality test_golden_cross_category_isolation -- --ignored --nocapture
#[ignore]
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_golden_cross_category_isolation(pool: PgPool) {
    let provider_lock = get_provider().await;
    let provider = provider_lock.lock().await;

    let store = PostgresMemoryStore::from_pool(pool.clone())
        .await
        .expect("Failed to create store");

    let golden = load_golden_queries();

    // Seed all golden memories with real embeddings
    for entry in &golden {
        let embedding = provider
            .embed(&entry.seed_content)
            .await
            .expect("Failed to embed seed content");
        store_with_real_embedding(&store, &pool, &entry.seed_content, &embedding).await;
    }

    // Find the first preference-category query
    let pref_entry = golden
        .iter()
        .find(|e| e.category == "preference")
        .expect("Golden dataset must include at least one 'preference' category entry");

    let query_emb = provider
        .embed(&pref_entry.query)
        .await
        .expect("Failed to embed preference query");
    let query_vector = Vector::from(query_emb);

    let hits = store
        .hybrid_search(
            &pref_entry.query,
            Some(&query_vector),
            5,
            None,
            None,
            None,
            Some(60.0),
            Some(60.0),
            Some(40.0),
            None,
            None,
            None, // project
        )
        .await
        .expect("hybrid_search failed");

    assert!(
        !hits.is_empty(),
        "Preference query '{}' returned no results",
        pref_entry.query
    );

    let top_content = &hits[0].memory.content;
    assert!(
        top_content.contains(&pref_entry.expected_top_content),
        "Category isolation failed: preference query '{}'\n  Expected top to contain: '{}'\n  Got: '{}'",
        pref_entry.query,
        pref_entry.expected_top_content,
        top_content
    );

    println!(
        "PASS: preference query '{}' correctly returned: '{}'",
        pref_entry.query, top_content
    );
}
