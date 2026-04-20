//! Phase 24.75 chunk removal — get_memory_span ranking + offset tests.
//!
//! Flipped ON in Plan 24.75-04. Exercises the shared compute path used by the
//! MCP tool, HTTP handler, and CLI — all three delegate to
//! `transport::api::memory_span::compute_memory_span` so these tests lock the
//! semantics for every surface.
//!
//! Targets CHUNK-04 (topic-ranked span retrieval + valid byte offsets) from
//! 24.75-RESEARCH.md's Validation Architecture.

#![allow(clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use memcp::embedding::{EmbeddingError, EmbeddingProvider};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use memcp::transport::api::memory_span::compute_memory_span;
use sqlx::PgPool;
use tokio::sync::Mutex;

#[path = "common/builders.rs"]
mod builders;

/// Topic-aware mock embedding provider. Keyword-indicator embedding — each
/// dimension is a keyword presence flag. Cosine similarity picks the span whose
/// keywords overlap with the topic query the most.
///
/// We deliberately avoid the local-embed feature here so CI without ML runtimes
/// still exercises ranking logic end-to-end.
struct KeywordEmbedder {
    keywords: Vec<&'static str>,
}

impl KeywordEmbedder {
    fn new() -> Self {
        Self {
            keywords: vec![
                "authentication",
                "auth",
                "login",
                "password",
                "billing",
                "invoice",
                "payment",
                "shipping",
                "delivery",
                "address",
            ],
        }
    }
}

#[async_trait]
impl EmbeddingProvider for KeywordEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let lower = text.to_lowercase();
        let v: Vec<f32> = self
            .keywords
            .iter()
            .map(|kw| if lower.contains(kw) { 1.0 } else { 0.0 })
            .collect();
        Ok(v)
    }

    fn model_name(&self) -> &str {
        "keyword-mock"
    }

    fn dimension(&self) -> usize {
        self.keywords.len()
    }
}

// Each topic paragraph is sized >2048 chars so span_sentences puts it in its
// own group without spillover from the next topic. This keeps per-topic
// ranking deterministic against SPAN_MAX_CHARS=2048 / overlap_sentences=1.
fn auth_paragraph() -> String {
    let base = "Authentication flow. The login subsystem validates credentials \
                by hashing the user-supplied password with argon2 and comparing \
                the result to the stored hash. When authentication succeeds we \
                mint a short-lived access token and a longer refresh token. The \
                refresh token rotates on every login to limit replay exposure. \
                Password reset triggers a signed email link, also tied to the \
                authentication subsystem. ";
    base.repeat(6)
}

fn billing_paragraph() -> String {
    let base = "Billing and invoices. Every paid plan generates a monthly \
                invoice line, and card-on-file payment happens three days \
                before the billing period ends. Failed payment retries twice \
                before the account moves to a delinquent state; each retry \
                re-emails the customer with the updated invoice. Billing \
                reports aggregate by project for multi-tenant customers. ";
    base.repeat(6)
}

fn shipping_paragraph() -> String {
    let base = "Shipping and delivery. Physical goods ship from the nearest \
                regional warehouse, and the shipping address gets validated \
                against the postal carrier's geocoding service at checkout. \
                Delivery tracking updates propagate back through the shipping \
                webhook into the customer's order page. Failed delivery \
                attempts generate a shipping exception row. ";
    base.repeat(6)
}

async fn seed_long_memory(store: &PostgresMemoryStore) -> (String, String) {
    let content = format!(
        "{}\n\n{}\n\n{}",
        auth_paragraph(),
        billing_paragraph(),
        shipping_paragraph()
    );
    let input = builders::MemoryBuilder::new()
        .content(&content)
        .project("test")
        .build();
    let mem = store.store(input).await.expect("seed memory");
    (mem.id, content)
}

/// Seed a ~3-5kB memory with three distinct topic paragraphs, query for
/// "authentication" via `get_memory_span`, and assert the returned span carries
/// authentication content (not billing, not shipping) with valid byte offsets.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_topic_ranked_span(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let (id, full_content) = seed_long_memory(&store).await;

    let provider: Arc<dyn EmbeddingProvider + Send + Sync> = Arc::new(KeywordEmbedder::new());
    let cache = Arc::new(Mutex::new(HashMap::new()));
    let store_arc: Arc<dyn MemoryStore + Send + Sync> = Arc::new(store);

    let resp = compute_memory_span(
        store_arc,
        provider,
        cache,
        &id,
        "authentication login credentials",
    )
    .await
    .expect("compute_memory_span ok");

    // Returned span content must carry auth keywords — not billing / shipping.
    let lower = resp.content.to_lowercase();
    assert!(
        lower.contains("authentication") || lower.contains("login"),
        "Expected auth keywords in span content, got: {}",
        &resp.content[..resp.content.len().min(200)]
    );
    assert!(
        !lower.contains("invoice") && !lower.contains("shipping"),
        "Auth-topic span should not contain billing/shipping keywords"
    );

    // Byte offsets must point into the auth region of the parent memory.
    assert!(resp.span.start < resp.span.end, "start must precede end");
    assert!(
        resp.span.end <= full_content.len(),
        "end must fit within parent content length"
    );
    let auth_start = full_content.to_lowercase().find("authentication").unwrap();
    let first_billing = full_content.to_lowercase().find("billing").unwrap();
    assert!(
        resp.span.start >= auth_start && resp.span.start < first_billing,
        "span.start ({}) must land in auth region [{} .. {})",
        resp.span.start,
        auth_start,
        first_billing
    );
}

/// Structural invariants on any span returned by `compute_memory_span`:
///   - `0 <= span.start < span.end <= memory.content.len()`
///   - `memory.content[span.start..span.end] == returned.content` when the
///     splitter produced a substring (common case; short-content fallback
///     returns the whole memory with start=0, end=len which still satisfies
///     the slice invariant).
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_memory_span_offsets_valid(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool).await.unwrap();
    let (id, full_content) = seed_long_memory(&store).await;

    let provider: Arc<dyn EmbeddingProvider + Send + Sync> = Arc::new(KeywordEmbedder::new());
    let cache = Arc::new(Mutex::new(HashMap::new()));
    let store_arc: Arc<dyn MemoryStore + Send + Sync> = Arc::new(store);

    let resp = compute_memory_span(store_arc, provider, cache, &id, "billing invoice payment")
        .await
        .expect("compute_memory_span ok");

    assert!(resp.span.start < resp.span.end, "start < end");
    assert!(
        resp.span.end <= full_content.len(),
        "span.end ({}) must be <= content len ({})",
        resp.span.end,
        full_content.len()
    );

    // Parent slice at the returned byte range must equal the returned content.
    let slice = &full_content[resp.span.start..resp.span.end];
    assert_eq!(
        slice, resp.content,
        "memory.content[start..end] must equal returned span content"
    );
}
