//! Migration 028 orchestrator: collapse chunk rows back into their parents.
//!
//! Run this binary BEFORE migration 028 SQL applies. See
//! `.planning/phases/24.75-chunk-semantics-rethink/24.75-RESEARCH.md` Risk R-1:
//! the vectorization (re-embed) step is async network I/O and cannot live inside
//! a `sqlx::migrate!` migration. This binary performs the embedding work in
//! short per-parent transactions (Pitfall 6), then does a single final
//! `DELETE FROM memories WHERE parent_id IS NOT NULL`.
//!
//! A1 decision (see `24.75-A1-PROBE.md`): A1-UNDECIDABLE-EMPTY. Source-level
//! analysis confirms parents hold the full pre-chunking content (auto_store
//! creates the parent with full content before fan-out, never overwrites it),
//! so the preferred path is re-embed parent.content directly. The orchestrator
//! aborts with a clean error if any parent row has empty content — that is the
//! cheap belt-and-braces the empty-dev-DB probe could not supply (see A1-PROBE
//! "Guardrail" bullet).
//!
//! Usage:
//!   cargo run --features local-embed --bin migrate_028_collapse_chunks -- --dry-run
//!   cargo run --features local-embed --bin migrate_028_collapse_chunks
//!
//! Idempotent: re-running on an already-collapsed DB exits 0 with zero work.

#![cfg(feature = "local-embed")]

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;

use memcp::config::Config;
use memcp::daemon::create_embedding_provider;
use memcp::embedding::{model_dimension, EmbeddingProvider};
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{Memory, MemoryStore};

/// Default per-parent transaction pool size. Small — we never hold more than
/// one short transaction at a time from the main loop.
const DEFAULT_MAX_CONNECTIONS: u32 = 4;

/// CLI args.
#[derive(Parser, Debug)]
#[command(
    name = "migrate_028_collapse_chunks",
    about = "Phase 24.75 migration 028 orchestrator — collapses chunk rows into \
             their parents and re-embeds each parent. Run BEFORE applying \
             migration 028 SQL."
)]
struct Args {
    /// Database URL (defaults to $DATABASE_URL).
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    /// Print the plan (parent IDs that would be processed) without mutating anything.
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Path for the append-only per-parent report file.
    #[arg(long, default_value = "data/migration_028_report.jsonl")]
    report_path: String,
}

/// One JSONL line appended per parent processed. Audit trail for the operator
/// (Research Open Question 4).
#[derive(Serialize)]
struct ReportLine {
    parent_id: String,
    chunk_count_collapsed: usize,
    new_embedding_dim: usize,
    reassembled: bool,
    timestamp: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let db_url = args
        .database_url
        .clone()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context("DATABASE_URL not set (pass --database-url or export DATABASE_URL)")?;

    let pool = PgPoolOptions::new()
        .max_connections(DEFAULT_MAX_CONNECTIONS)
        .connect(&db_url)
        .await
        .context("failed to connect to Postgres")?;

    // Load config to reuse the daemon's embedding provider constructor (honors
    // TOML + MEMCP_EMBEDDING__* env vars exactly like the live service).
    let config = Config::load().context("failed to load memcp config")?;
    let provider: Arc<dyn EmbeddingProvider + Send + Sync> = create_embedding_provider(&config)
        .await
        .context("failed to create embedding provider")?;

    // --- Pitfall 5: dimension guard -----------------------------------------
    // Refuse to proceed if the provider's advertised dimension doesn't match
    // the model registry's known dimension. Silent dimension drift would write
    // wrong-size vectors to the schema.
    let expected_dim = model_dimension(provider.model_name()).with_context(|| {
        format!(
            "unknown embedding model '{}' — refusing to proceed. \
             Add it to intelligence::embedding::model_dimension() or switch models.",
            provider.model_name()
        )
    })?;
    if provider.dimension() != expected_dim {
        anyhow::bail!(
            "Embedding dimension mismatch: provider reports {}, registry expects {} for model {}. \
             Refusing to proceed. Run `memcp embed switch-model` or reconcile config first.",
            provider.dimension(),
            expected_dim,
            provider.model_name()
        );
    }

    // --- Idempotency probe --------------------------------------------------
    // If no chunk rows exist, this orchestrator is a no-op. Safe to apply the
    // DDL migration directly. Returning 0 means a rerun after a successful
    // previous run also exits cleanly.
    let parent_ids: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT parent_id \
         FROM memories \
         WHERE parent_id IS NOT NULL AND deleted_at IS NULL",
    )
    .fetch_all(&pool)
    .await
    .context("failed to query parent ids for chunks")?;

    if parent_ids.is_empty() {
        eprintln!(
            "No chunk rows found — migration 028 orchestrator is a no-op. \
             Safe to apply SQL migration 028."
        );
        return Ok(());
    }

    eprintln!(
        "Found {} parents with chunk rows. Embedding model: {} (dim {}).",
        parent_ids.len(),
        provider.model_name(),
        provider.dimension()
    );

    if args.dry_run {
        eprintln!("--dry-run: printing parent ids, no mutation.");
        for pid in &parent_ids {
            println!("{}", pid);
        }
        return Ok(());
    }

    // Build a PostgresMemoryStore for read helpers (get / get_chunks_by_parent).
    // It maintains its own pool; our `pool` above is used for the write
    // statements so transactions stay scoped to this orchestrator.
    let store = PostgresMemoryStore::new(&db_url, false)
        .await
        .context("failed to build PostgresMemoryStore")?;

    run_collapse(&pool, &store, provider.as_ref(), &parent_ids, &args.report_path).await
}

/// Main loop: for each parent, reassemble-or-trust parent content, re-embed,
/// short transaction to update, log to report. Finishes with a single
/// DELETE FROM memories WHERE parent_id IS NOT NULL.
async fn run_collapse(
    pool: &PgPool,
    store: &PostgresMemoryStore,
    provider: &(dyn EmbeddingProvider + Send + Sync),
    parent_ids: &[String],
    report_path: &str,
) -> Result<()> {

    let pb = ProgressBar::new(parent_ids.len() as u64);
    pb.set_style(
        ProgressStyle::with_template("{pos}/{len} {bar:40} {eta}")
            .context("progress bar template")?,
    );

    for parent_id in parent_ids {
        // Last use of get_chunks_by_parent before Plan 03 removes the helper.
        let chunks = store.get_chunks_by_parent(parent_id).await?;
        let parent = store.get(parent_id).await?;

        // A1 guardrail (A1-UNDECIDABLE-EMPTY path from 24.75-A1-PROBE.md):
        // parent.content is authoritative by construction (auto_store inserts
        // full content before fan-out). If a parent is empty with chunks
        // present, A1 is violated on this DB — bail so we don't write a
        // zero-length embedding over the schema.
        if parent.content.is_empty() {
            anyhow::bail!(
                "A1 violation: parent {} has empty content but {} chunk rows. \
                 Refusing to proceed — this would destroy data. \
                 See 24.75-A1-PROBE.md for details.",
                parent_id,
                chunks.len()
            );
        }

        let full_content = detect_and_reassemble(&parent, &chunks);
        let reassembled = full_content != parent.content;

        let embedding = provider
            .embed(&full_content)
            .await
            .with_context(|| format!("embed failed for parent {}", parent_id))?;

        // --- Pitfall 6: short per-parent transaction -------------------------
        let mut tx = pool.begin().await?;
        if reassembled {
            sqlx::query("UPDATE memories SET content = $1 WHERE id = $2")
                .bind(&full_content)
                .bind(parent_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "UPDATE memory_embeddings \
             SET embedding = $1::vector, updated_at = NOW() \
             WHERE memory_id = $2 AND is_current = true",
        )
        .bind(pgvector::Vector::from(embedding.clone()))
        .bind(parent_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        append_report(
            report_path,
            &ReportLine {
                parent_id: parent_id.clone(),
                chunk_count_collapsed: chunks.len(),
                new_embedding_dim: embedding.len(),
                reassembled,
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
        )?;
        pb.inc(1);
    }
    pb.finish();

    // Final sweep: one statement, no embedding in flight. After this returns 0
    // rows, the DDL step (migration 028) is safe to apply.
    let deleted = sqlx::query("DELETE FROM memories WHERE parent_id IS NOT NULL")
        .execute(pool)
        .await?
        .rows_affected();
    eprintln!(
        "Deleted {} chunk rows. Next step: run `memcp daemon` to apply SQL migration 028.",
        deleted
    );
    Ok(())
}

/// Reassemble full content from chunks if the parent row does NOT already hold
/// it (A1-REFUTED shape). In A1-CONFIRMED / A1-UNDECIDABLE-EMPTY shape the
/// parent is authoritative and this returns `parent.content` unchanged.
///
/// Heuristic: if `parent.content.len()` is within the range where it could be
/// "chunk 0 only" (well under the chunk total minus header overhead), we
/// reassemble; otherwise we trust the parent. For the dev DB this evaluates to
/// "trust parent" every time because chunks are empty.
///
/// Exposed as `pub(crate)` so the integration test at
/// `crates/memcp-core/tests/chunk_removal_test.rs::test_migration_028_collapse`
/// can exercise it directly via the binary's module path.
pub fn detect_and_reassemble(parent: &Memory, chunks: &[Memory]) -> String {
    if chunks.is_empty() {
        return parent.content.clone();
    }
    let chunk_total: usize = chunks.iter().map(|c| c.content.len()).sum();
    // Per-chunk header overhead: "[From: \"title\", part N/M]\n" — ~40 chars
    // per chunk is a conservative average across typical titles.
    let header_overhead = chunks.len().saturating_mul(40);

    // If the parent length is in the same order as the reassembled body
    // (chunk_total minus all headers), the parent already holds full content.
    // Otherwise the parent is a prefix/preview and we must reassemble.
    if parent.content.len().saturating_add(header_overhead) >= chunk_total {
        return parent.content.clone();
    }

    // Reassemble path (A1-REFUTED): strip the "[From: ...]\n" header line from
    // each chunk body, then concatenate in chunk_index order. Chunks are
    // already ordered by chunk_index ASC by get_chunks_by_parent.
    let mut out = String::with_capacity(chunk_total);
    for c in chunks {
        let body = strip_context_header(&c.content);
        out.push_str(body);
    }
    out
}

/// Strip a leading `[From: "...", part N/M]\n` line produced by
/// `pipeline::chunking::splitter::make_context_header`. If no such line is
/// present, return the content unchanged.
fn strip_context_header(content: &str) -> &str {
    match content.split_once('\n') {
        Some((head, rest))
            if head.starts_with("[From:") && head.trim_end().ends_with(']') =>
        {
            rest
        }
        _ => content,
    }
}

fn append_report(path: &str, line: &ReportLine) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create report dir {}", parent.display()))?;
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open report file {}", path))?;
    writeln!(f, "{}", serde_json::to_string(line)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn mk_memory(id: &str, content: &str, parent_id: Option<&str>, chunk_index: Option<i32>) -> Memory {
        Memory {
            id: id.to_string(),
            content: content.to_string(),
            type_hint: "note".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
            embedding_status: "pending".to_string(),
            extracted_entities: None,
            extracted_facts: None,
            extraction_status: "pending".to_string(),
            is_consolidated_original: false,
            consolidated_into: None,
            actor: None,
            actor_type: "system".to_string(),
            audience: "global".to_string(),
            parent_id: parent_id.map(str::to_string),
            chunk_index,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: 0.5,
            session_id: None,
            agent_role: None,
            write_path: None,
            metadata: json!({}),
            abstract_text: None,
            overview_text: None,
            abstraction_status: "skipped".to_string(),
            knowledge_tier: "raw".to_string(),
            source_ids: None,
            reply_to_id: None,
        }
    }

    #[test]
    fn parent_with_full_content_is_trusted() {
        // A1-CONFIRMED shape: parent.content has full pre-chunking content.
        let parent = mk_memory("p", "hello world from memcp", None, None);
        let chunks = vec![
            mk_memory("c0", "[From: \"t\", part 1/2]\nhello world", Some("p"), Some(0)),
            mk_memory("c1", "[From: \"t\", part 2/2]\n from memcp", Some("p"), Some(1)),
        ];
        let out = detect_and_reassemble(&parent, &chunks);
        assert_eq!(out, parent.content);
    }

    #[test]
    fn parent_preview_triggers_reassembly() {
        // A1-REFUTED shape: parent.content is a short preview; chunks carry
        // the real payload. Body total dwarfs (parent.len + per-chunk headers).
        let parent = mk_memory("p", "preview only", None, None);
        let chunks = vec![
            mk_memory(
                "c0",
                "[From: \"t\", part 1/2]\nTHIS IS THE FIRST REAL CHUNK BODY OF THE FULL CONTENT",
                Some("p"),
                Some(0),
            ),
            mk_memory(
                "c1",
                "[From: \"t\", part 2/2]\nAND THIS IS THE SECOND REAL CHUNK BODY OF THE FULL CONTENT",
                Some("p"),
                Some(1),
            ),
        ];
        let out = detect_and_reassemble(&parent, &chunks);
        assert!(out.contains("THIS IS THE FIRST"));
        assert!(out.contains("AND THIS IS THE SECOND"));
        assert!(!out.starts_with("[From:"));
    }

    #[test]
    fn reassembly_strips_context_headers() {
        assert_eq!(
            strip_context_header("[From: \"title\", part 1/3]\nreal body"),
            "real body"
        );
        assert_eq!(strip_context_header("no header here"), "no header here");
        assert_eq!(
            strip_context_header("[From: \"x\", part 2/2]\n"),
            ""
        );
    }

    #[test]
    fn no_chunks_returns_parent_content() {
        let parent = mk_memory("p", "solo memory", None, None);
        let out = detect_and_reassemble(&parent, &[]);
        assert_eq!(out, "solo memory");
    }
}
