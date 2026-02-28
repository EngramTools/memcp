//! 100k memory stress test — validates search, pagination, and GC performance at scale.
//!
//! Excluded from normal CI via `#[ignore]`. Run locally with:
//!
//! ```
//! DATABASE_URL=postgres://memcp:memcp@localhost:5433/memcp \
//!   cargo test stress_100k -- --ignored --nocapture
//! ```
//!
//! Expected timing (on modern hardware):
//! - Bulk insert (90k raw + 10k store):  30–120s
//! - BM25 search:                        <2s
//! - List page:                          <200ms
//! - GC candidates:                      <5s
//! - Soft delete 100:                    <1s

mod common;
use common::builders::MemoryBuilder;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{ListFilter, MemoryStore};
use sqlx::PgPool;
use std::time::Instant;

/// Content variety pool — rotated so each record has unique content.
const VARIED_CONTENT: &[&str] = &[
    "The user prefers dark mode in all editors and terminals",
    "Rust ownership rules prevent data races at compile time",
    "PostgreSQL tsvector enables fast full-text search without external services",
    "The project uses fastembed for local embedding generation",
    "Agent memory should be structured with type_hint for retrieval quality",
    "Salience decay models FSRS spaced-repetition for memory pruning",
    "BM25 and vector search are fused with Reciprocal Rank Fusion",
    "The daemon runs background workers for embedding and extraction",
    "CLI commands are short-lived; daemon hosts long-running processes",
    "GC prunes memories with stability below the configured threshold",
    "Cursor-based pagination avoids offset performance degradation at scale",
    "Tag-based filtering allows scoped memory retrieval by topic",
    "The user wants concise answers without restating the question",
    "OpenAI embedding API supports text-embedding-3-small and text-embedding-3-large",
    "Config is layered: defaults into memcp.toml into environment variables",
    "Hard purge cascades to memory_embeddings and memory_salience tables",
    "Auto-summarization uses Ollama or OpenAI to compress long assistant responses",
    "Provenance fields actor actor_type audience are stored for each memory",
    "Topic exclusion filters ingestion-time content using regex and semantic matching",
    "The MCP protocol uses JSON-RPC over stdio for tool invocation",
];

/// Type hint distribution: 40% fact, 25% preference, 20% decision, 15% instruction.
fn type_hint_for(i: usize) -> &'static str {
    match i % 20 {
        0..=7 => "fact",
        8..=12 => "preference",
        13..=15 => "decision",
        _ => "instruction",
    }
}

/// Tag assignment: ~30% no tags, ~40% one tag, ~30% two tags.
fn tags_for(i: usize) -> Option<serde_json::Value> {
    let tag_pool = ["rust", "memory", "search", "config", "daemon", "cli", "gc", "embedding"];
    match i % 10 {
        0..=2 => None, // 30% no tags
        3..=6 => Some(serde_json::json!([tag_pool[i % tag_pool.len()]])), // 40% one tag
        _ => Some(serde_json::json!([
            tag_pool[i % tag_pool.len()],
            tag_pool[(i + 3) % tag_pool.len()],
        ])), // 30% two tags
    }
}

#[ignore]
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn stress_100k_memories(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // ─── Phase 1: Bulk insert 100k records ───────────────────────────────────
    //
    // 90k via raw SQL batch inserts (100 rows per statement, 4 params per row)
    // for speed. Tags stored as JSON; NULL tags use SQL NULL.
    // 10k via store() to exercise the full application path including
    // content_hash computation, idempotency key registration, etc.
    //
    // Batch size = 100 rows × 4 params = 400 params per query (well within 65535 limit).

    println!("\n=== Stress Test: 100k memories ===");
    println!("Phase 1: Bulk inserting 100k records...");

    let insert_start = Instant::now();

    // Raw SQL path: 900 batches × 100 rows = 90k records
    //
    // Each row uses 5 params: content, type_hint, source, tags (as jsonb), content_hash.
    // content_hash is pre-computed in Rust using FNV-1a (same algorithm as production code).
    // Batch size = 100 rows × 5 params = 500 params per query (well within 65535 pg limit).
    for batch in 0..900usize {
        let batch_size = 100usize;
        let mut value_clauses: Vec<String> = Vec::with_capacity(batch_size);
        let mut contents: Vec<String> = Vec::with_capacity(batch_size);
        let mut type_hints: Vec<String> = Vec::with_capacity(batch_size);
        let mut sources: Vec<String> = Vec::with_capacity(batch_size);
        let mut tags_values: Vec<Option<serde_json::Value>> = Vec::with_capacity(batch_size);
        let mut hashes: Vec<String> = Vec::with_capacity(batch_size);

        for row in 0..batch_size {
            let i = batch * batch_size + row;
            let p = row * 5 + 1; // each row uses 5 params
            value_clauses.push(format!(
                "(gen_random_uuid()::text, ${}, ${}, ${}, ${}::jsonb, 'agent', 'global', NOW(), NOW(), false, ${}, NULL, 'done')",
                p, p + 1, p + 2, p + 3, p + 4
            ));
            let content = format!(
                "Stress test memory number {}: {}",
                i,
                VARIED_CONTENT[i % VARIED_CONTENT.len()]
            );
            hashes.push(fnv1a_hex(&content));
            contents.push(content);
            type_hints.push(type_hint_for(i).to_string());
            sources.push(format!("stress-batch-{}", batch / 10));
            tags_values.push(tags_for(i));
        }

        let sql = format!(
            "INSERT INTO memories \
             (id, content, type_hint, source, tags, actor_type, audience, \
              created_at, updated_at, is_consolidated_original, content_hash, deleted_at, embedding_status) \
             VALUES {}",
            value_clauses.join(", ")
        );

        let mut q = sqlx::query(&sql);
        for row in 0..batch_size {
            q = q
                .bind(&contents[row])
                .bind(&type_hints[row])
                .bind(&sources[row])
                .bind(&tags_values[row])
                .bind(&hashes[row]);
        }
        q.execute(&pool).await.expect("Batch insert failed");

        if batch % 100 == 99 {
            println!("  Inserted {}k records...", (batch + 1) / 10);
        }
    }

    // Store path: 10k records via store() — exercises full application code path
    println!("  Inserting final 10k via store()...");
    for i in 90_000..100_000usize {
        let content = format!(
            "Stress test memory number {}: {}",
            i,
            VARIED_CONTENT[i % VARIED_CONTENT.len()]
        );
        let type_hint = type_hint_for(i);

        let mut create = MemoryBuilder::new()
            .content(&content)
            .type_hint(type_hint)
            .source("stress-store-path")
            .build();

        // Set tags via direct field assignment
        create.tags = tags_for(i).and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        store.store(create).await.expect("store() failed during stress insert");

        if i % 2000 == 1999 {
            println!("  store() path: {}k done...", (i - 90_000 + 1) / 1000 + 90);
        }
    }

    let insert_ms = insert_start.elapsed().as_millis();
    println!("  Bulk insert done in {}ms ({:.1}s)", insert_ms, insert_ms as f64 / 1000.0);

    // ─── Phase 2: Search performance ─────────────────────────────────────────
    //
    // BM25-only search (no embedding) — exercises tsvector GIN index.

    println!("\nPhase 2: BM25 search performance...");
    let search_start = Instant::now();

    let search_results = store
        .hybrid_search(
            "stress test memory",
            None,      // no embedding — BM25 only
            10,
            None,      // created_after
            None,      // created_before
            None,      // tags
            Some(1.0), // bm25_k — enable BM25 leg
            None,      // vector_k — disable vector leg
            None,      // symbolic_k — disable symbolic leg
            None,      // source
            None,      // audience
            None,      // workspace
        )
        .await
        .expect("hybrid_search failed");

    let search_ms = search_start.elapsed().as_millis();
    println!("  Search returned {} results in {}ms", search_results.len(), search_ms);
    assert!(
        search_ms < 2000,
        "BM25 search took {}ms — expected <2000ms at 100k scale",
        search_ms
    );

    // ─── Phase 3: Paginated list performance ─────────────────────────────────
    //
    // Cursor-based list pagination — exercises the memories table created_at index.

    println!("\nPhase 3: Paginated list performance...");
    let list_p1_start = Instant::now();

    let page1 = store
        .list(ListFilter {
            limit: 50,
            ..Default::default()
        })
        .await
        .expect("list() page 1 failed");

    let list_p1_ms = list_p1_start.elapsed().as_millis();
    println!(
        "  Page 1: {} memories in {}ms (has_next: {})",
        page1.memories.len(),
        list_p1_ms,
        page1.next_cursor.is_some()
    );
    assert!(
        list_p1_ms < 200,
        "List page 1 took {}ms — expected <200ms",
        list_p1_ms
    );
    assert_eq!(page1.memories.len(), 50, "Expected 50 results on page 1");

    // Page 2 using cursor from page 1
    let list_p2_start = Instant::now();

    let page2 = store
        .list(ListFilter {
            limit: 50,
            cursor: page1.next_cursor.clone(),
            ..Default::default()
        })
        .await
        .expect("list() page 2 failed");

    let list_p2_ms = list_p2_start.elapsed().as_millis();
    println!("  Page 2: {} memories in {}ms", page2.memories.len(), list_p2_ms);
    assert!(
        list_p2_ms < 200,
        "List page 2 took {}ms — expected <200ms",
        list_p2_ms
    );

    // ─── Phase 4: GC candidate retrieval ─────────────────────────────────────
    //
    // Tests the salience LEFT JOIN query at scale.
    // No salience data seeded — exercises the query with empty joins.
    // All memories have default stability (1.0) via COALESCE, so with
    // threshold 0.3 we expect 0 candidates (1.0 >= 0.3 not less than).
    // This still exercises the full query plan at 100k scale.

    println!("\nPhase 4: GC candidate retrieval...");
    let gc_start = Instant::now();

    let candidates = store
        .get_gc_candidates(1.5, 0, 1000) // threshold 1.5 > default 1.0 — finds all memories
        .await
        .expect("get_gc_candidates failed");

    let gc_ms = gc_start.elapsed().as_millis();
    println!(
        "  GC returned {} candidates in {}ms (salience threshold 1.5)",
        candidates.len(),
        gc_ms
    );
    assert!(
        gc_ms < 5000,
        "GC candidate retrieval took {}ms — expected <5000ms",
        gc_ms
    );

    // ─── Phase 5: Soft delete performance ────────────────────────────────────
    //
    // Soft-delete 100 memories — exercises UPDATE on indexed deleted_at column.

    println!("\nPhase 5: Soft delete 100 memories...");

    // Fetch 100 memory IDs to soft-delete
    let sample = store
        .list(ListFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("list() for soft-delete sample failed");

    let ids: Vec<String> = sample.memories.iter().map(|m| m.id.clone()).collect();
    assert!(!ids.is_empty(), "Expected memories to soft-delete");

    let delete_start = Instant::now();
    let deleted_count = store
        .soft_delete_memories(&ids)
        .await
        .expect("soft_delete_memories failed");
    let delete_ms = delete_start.elapsed().as_millis();

    println!("  Soft-deleted {} memories in {}ms", deleted_count, delete_ms);
    assert!(
        delete_ms < 1000,
        "Soft delete 100 took {}ms — expected <1000ms",
        delete_ms
    );

    // ─── Summary ─────────────────────────────────────────────────────────────

    println!("\n=== Stress Test Results (100k memories) ===");
    println!("  Bulk insert:     {}ms ({:.1}s)", insert_ms, insert_ms as f64 / 1000.0);
    println!("  Search (BM25):   {}ms", search_ms);
    println!("  List page 1:     {}ms", list_p1_ms);
    println!("  List page 2:     {}ms", list_p2_ms);
    println!("  GC candidates:   {}ms", gc_ms);
    println!("  Soft delete 100: {}ms", delete_ms);
    println!("===========================================");
}

/// FNV-1a 64-bit hash — mirrors the production content_hash algorithm in postgres.rs.
///
/// Used in bulk inserts to generate unique content_hash values without calling
/// any PostgreSQL extension functions (pgcrypto / sha256 not available in test DBs).
fn fnv1a_hex(content: &str) -> String {
    let mut hash: u64 = 14695981039346656037;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{:016x}", hash)
}
