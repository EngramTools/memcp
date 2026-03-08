//! Corpus seeding for load tests.
//!
//! Seeds a Postgres database with N synthetic memories and pre-computed embeddings
//! so the load test driver can immediately issue search/recall operations without
//! waiting for the embedding pipeline to process new stores.

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use sqlx::PgPool;
use std::time::Instant;

// ─── Content Template Pool ────────────────────────────────────────────────────

/// Varied content templates for seeded memories.
///
/// Mix of code patterns, user preferences, architectural facts, project conventions,
/// and bug fix notes — varied length so the FTS and vector indexes see realistic data.
const CONTENT_TEMPLATES: &[&str] = &[
    // Code patterns & decisions
    "Use async/await for all I/O operations to avoid blocking the Tokio runtime thread pool",
    "Prefer Arc<Mutex<T>> over Rc<RefCell<T>> in async contexts to avoid Send + Sync violations",
    "Always propagate errors with the ? operator rather than unwrap() in production code",
    "Use structured logging with tracing::instrument for automatic span propagation across async boundaries",
    "Batch database inserts using UNNEST or value lists instead of N individual INSERT statements for performance",
    "Index Postgres JSONB columns with GIN indexes when filtering on array contents",
    "Use pgvector's <=> cosine distance operator — it normalizes internally, no need to pre-normalize embeddings",
    "Apply EXPLAIN ANALYZE before shipping any query that touches more than 10k rows",
    // User preferences
    "Prefers dark mode in all editors, terminals, and web applications",
    "Wants concise answers without restating the question or summarizing what was just said",
    "Prefers Rust over TypeScript for systems-level work due to compile-time safety guarantees",
    "Likes to see concrete examples with code snippets rather than abstract explanations",
    "Prefers pnpm over npm for TypeScript projects — faster installs and strict hoisting rules",
    // Architectural facts
    "PostgreSQL tsvector enables full-text search without requiring an external search service like Elasticsearch",
    "Reciprocal Rank Fusion (RRF) merges BM25 and vector search rankings without requiring score normalization",
    "fastembed generates 384-dimensional embeddings locally using ONNX Runtime — no API key required",
    "The Tokio async runtime uses M:N threading — N OS threads multiplex across M async tasks",
    "pgvector's HNSW index supports approximate nearest-neighbor search at sub-linear query time",
    "GovernorLayer from the tower_governor crate implements token-bucket rate limiting per IP address",
    // Project conventions
    "Always use structured logging with request_id spans for distributed tracing across service boundaries",
    "CLI commands are short-lived processes; long-running background work belongs in the daemon",
    "Migrations live in crates/memcp-core/migrations/ and run automatically via sqlx migrate on startup",
    "Test helpers live in tests/common/ — reuse MemoryBuilder rather than constructing CreateMemoryRequest manually",
    "Environment variables override memcp.toml which overrides compiled defaults — figment layering order",
    // Bug fix notes
    "Fixed race condition in the embedding pipeline by adding a Semaphore to cap concurrent embedding requests",
    "Fixed off-by-one in cursor-based pagination: cursor is exclusive, not inclusive — use created_at < cursor",
    "Fixed degenerate cosine similarity from zero vectors by normalizing embeddings before insertion",
    "Resolved missing tsvector update on bulk INSERT by running UPDATE memories SET search_tsv after batch",
    "Fixed GC stability threshold comparison: use < not <= so memories at exactly threshold are retained",
    "Resolved duplicate content_hash collision by including source field in the hash computation input",
];

/// Type hint distribution matching production data distribution.
fn type_hint_for(i: usize) -> &'static str {
    match i % 20 {
        0..=7 => "fact",        // 40%
        8..=12 => "preference", // 25%
        13..=15 => "decision",  // 15% (rounds with instruction to fill 20 slots)
        _ => "instruction",     // 20%
    }
}

/// Tag assignment: 30% no tags, 40% one tag, 30% two tags.
fn tags_for(i: usize) -> Option<serde_json::Value> {
    let tag_pool = [
        "rust", "memory", "search", "config", "daemon", "cli", "gc", "embedding",
        "postgres", "api", "performance", "async",
    ];
    match i % 10 {
        0..=2 => None,
        3..=6 => Some(serde_json::json!([tag_pool[i % tag_pool.len()]])),
        _ => Some(serde_json::json!([
            tag_pool[i % tag_pool.len()],
            tag_pool[(i + 4) % tag_pool.len()],
        ])),
    }
}

/// Generate a random 384-dimensional unit vector for cosine similarity search.
///
/// Uses `rand::thread_rng()` to generate random f32 values, then normalizes
/// to unit length so cosine distance produces non-trivial, varied results.
fn random_unit_vector() -> Vec<f32> {
    let mut rng = rand::thread_rng();
    let mut v: Vec<f32> = (0..384).map(|_| rng.gen::<f32>() * 2.0 - 1.0).collect();

    let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for x in &mut v {
            *x /= magnitude;
        }
    }
    v
}

/// Format a 384-dim vector as a Postgres array literal `[f1,f2,...,f384]`.
fn format_pg_vector(v: &[f32]) -> String {
    let inner: Vec<String> = v.iter().map(|x| format!("{}", x)).collect();
    format!("[{}]", inner.join(","))
}

/// FNV-1a 64-bit hash — mirrors the production content_hash algorithm.
///
/// Generates a unique content_hash without requiring pgcrypto.
fn fnv1a_hex(content: &str) -> String {
    let mut hash: u64 = 14695981039346656037;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{:016x}", hash)
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Seed `count` memories with pre-computed embeddings into the Postgres database.
///
/// Distributes memories across `num_projects` project namespaces (round-robin) to
/// simulate multi-tenant load. All memories are inserted with `embedding_status = 'done'`
/// and corresponding rows in `memory_embeddings` so the load test can immediately
/// issue vector search requests without waiting for the background embedding worker.
///
/// Uses batch inserts of 100 rows per statement (matching the stress test pattern)
/// to stay well within Postgres's 65535 parameter limit.
pub async fn seed_corpus(pool: &PgPool, count: usize, num_projects: usize) -> Result<()> {
    let start = Instant::now();

    let pb = ProgressBar::new(count as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} memories ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    const BATCH_SIZE: usize = 100;

    // ── Phase 1: Insert memories ──────────────────────────────────────────────
    //
    // Column order: id, content, type_hint, tags, project, actor_type, audience,
    //               created_at, updated_at, is_consolidated_original, content_hash,
    //               deleted_at, embedding_status, source
    //
    // Each row binds 6 params: id, content, type_hint, tags, project, content_hash
    // Fixed literals for: actor_type, audience, timestamps, is_consolidated_original,
    //                      deleted_at, embedding_status, source
    //
    // 100 rows × 6 params = 600 params/batch (well within 65535 pg limit).

    let mut inserted_ids: Vec<String> = Vec::with_capacity(count);

    let full_batches = count / BATCH_SIZE;
    let remainder = count % BATCH_SIZE;

    for batch in 0..full_batches {
        let (ids, sql, params) = build_memory_batch(batch * BATCH_SIZE, BATCH_SIZE, num_projects);
        execute_memory_batch(pool, &sql, &params).await?;
        inserted_ids.extend(ids);
        pb.inc(BATCH_SIZE as u64);
    }

    if remainder > 0 {
        let (ids, sql, params) = build_memory_batch(full_batches * BATCH_SIZE, remainder, num_projects);
        execute_memory_batch(pool, &sql, &params).await?;
        inserted_ids.extend(ids);
        pb.inc(remainder as u64);
    }

    pb.finish_with_message("memories inserted");

    // ── Phase 2: Insert embeddings ────────────────────────────────────────────
    //
    // Each embedding row: memory_id (text), embedding (vector), model_name (text).
    // We insert embeddings in batches of 50 (vector literals are large strings).
    // Each row: 3 params → 50 × 3 = 150 params/batch.

    println!("  Seeding embeddings for {} memories...", count);
    let embed_pb = ProgressBar::new(count as u64);
    embed_pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/white} {pos}/{len} embeddings ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    const EMBED_BATCH: usize = 50;
    for chunk in inserted_ids.chunks(EMBED_BATCH) {
        let mut value_clauses: Vec<String> = Vec::with_capacity(chunk.len());
        let mut memory_ids: Vec<String> = Vec::with_capacity(chunk.len());
        let mut vectors: Vec<String> = Vec::with_capacity(chunk.len());

        for (j, id) in chunk.iter().enumerate() {
            let p = j * 3 + 1;
            value_clauses.push(format!("(${}, ${}::vector, ${})", p, p + 1, p + 2));
            memory_ids.push(id.clone());
            vectors.push(format_pg_vector(&random_unit_vector()));
        }

        let sql = format!(
            "INSERT INTO memory_embeddings (memory_id, embedding, model_name) VALUES {}",
            value_clauses.join(", ")
        );

        let mut q = sqlx::query(&sql);
        for j in 0..chunk.len() {
            q = q
                .bind(&memory_ids[j])
                .bind(&vectors[j])
                .bind("load-test-synthetic");
        }
        q.execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Embedding batch insert failed: {}", e))?;

        embed_pb.inc(chunk.len() as u64);
    }

    embed_pb.finish_with_message("embeddings seeded");

    // Update tsvector for all seeded memories in case trigger didn't fire on raw INSERT
    sqlx::query(
        "UPDATE memories SET search_tsv = to_tsvector('english', content) \
         WHERE search_tsv IS NULL AND source = 'load-test-batch'",
    )
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("tsvector update failed: {}", e))?;

    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "  {} memories seeded across {} projects in {:.1}s",
        count, num_projects, elapsed
    );

    Ok(())
}

/// Truncate the `memory_embeddings` and `memories` tables to reset state before a test run.
///
/// Uses CASCADE to handle any foreign key dependencies automatically.
pub async fn clear_corpus(pool: &PgPool) -> Result<()> {
    sqlx::query("TRUNCATE TABLE memory_embeddings, memories CASCADE")
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("TRUNCATE failed: {}", e))?;
    Ok(())
}

// ─── Internal Helpers ─────────────────────────────────────────────────────────

struct MemoryBatchParams {
    ids: Vec<String>,
    contents: Vec<String>,
    type_hints: Vec<String>,
    tags_values: Vec<Option<serde_json::Value>>,
    projects: Vec<String>,
    hashes: Vec<String>,
}

/// Build the SQL and parameter bindings for a batch of memory rows.
fn build_memory_batch(
    offset: usize,
    batch_size: usize,
    num_projects: usize,
) -> (Vec<String>, String, MemoryBatchParams) {
    let mut value_clauses: Vec<String> = Vec::with_capacity(batch_size);
    let mut ids: Vec<String> = Vec::with_capacity(batch_size);
    let mut contents: Vec<String> = Vec::with_capacity(batch_size);
    let mut type_hints: Vec<String> = Vec::with_capacity(batch_size);
    let mut tags_values: Vec<Option<serde_json::Value>> = Vec::with_capacity(batch_size);
    let mut projects: Vec<String> = Vec::with_capacity(batch_size);
    let mut hashes: Vec<String> = Vec::with_capacity(batch_size);

    for row in 0..batch_size {
        let i = offset + row;
        // 6 params per row: id, content, type_hint, tags, project, content_hash
        let p = row * 6 + 1;

        let id = uuid::Uuid::new_v4().to_string();
        let content = format!(
            "Load test memory #{}: {}",
            i,
            CONTENT_TEMPLATES[i % CONTENT_TEMPLATES.len()]
        );
        let project_name = format!("project-{}", i % num_projects.max(1));

        value_clauses.push(format!(
            "(${}::text, ${}, ${}, ${}::jsonb, ${}, \
             'agent', 'global', NOW(), NOW(), false, ${}, NULL, 'done', 'load-test-batch')",
            p, p + 1, p + 2, p + 3, p + 4, p + 5
        ));

        hashes.push(fnv1a_hex(&content));
        ids.push(id);
        contents.push(content);
        type_hints.push(type_hint_for(i).to_string());
        tags_values.push(tags_for(i));
        projects.push(project_name);
    }

    let sql = format!(
        "INSERT INTO memories \
         (id, content, type_hint, tags, project, actor_type, audience, \
          created_at, updated_at, is_consolidated_original, content_hash, deleted_at, \
          embedding_status, source) \
         VALUES {}",
        value_clauses.join(", ")
    );

    let params = MemoryBatchParams {
        ids: ids.clone(),
        contents,
        type_hints,
        tags_values,
        projects,
        hashes,
    };

    (ids, sql, params)
}

async fn execute_memory_batch(pool: &PgPool, sql: &str, params: &MemoryBatchParams) -> Result<()> {
    let mut q = sqlx::query(sql);
    for row in 0..params.ids.len() {
        q = q
            .bind(&params.ids[row])
            .bind(&params.contents[row])
            .bind(&params.type_hints[row])
            .bind(&params.tags_values[row])
            .bind(&params.projects[row])
            .bind(&params.hashes[row]);
    }
    q.execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Memory batch insert failed: {}", e))?;
    Ok(())
}
