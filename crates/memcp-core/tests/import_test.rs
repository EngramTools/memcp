//! Integration tests for the import pipeline.
//!
//! Tests use `#[sqlx::test]` for ephemeral database isolation.
//! Each test gets its own temporary database.

use std::io::Write;
use std::sync::Arc;

use memcp::import::{ImportEngine, ImportOpts};
use memcp::import::dedup;
use memcp::import::jsonl::JsonlReader;
use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::{ListFilter, MemoryStore};
use sqlx::PgPool;
use tempfile::NamedTempFile;

/// Helper: create a temp JSONL file with the given lines.
fn make_jsonl_file(lines: &[&str]) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(".jsonl").unwrap();
    for line in lines {
        writeln!(f, "{}", line).unwrap();
    }
    f
}

/// Sample JSONL lines (valid, content > 50 chars each).
const SAMPLE_MEMORIES: &[&str] = &[
    r#"{"content":"User prefers Rust over Go for backend services due to memory safety guarantees","type_hint":"preference","tags":["rust","languages"]}"#,
    r#"{"content":"Dark mode is the preferred UI theme for all development tools and editors","type_hint":"preference","tags":["ui","editor"]}"#,
    r#"{"content":"Always use async/await in Rust with Tokio for non-blocking IO operations in servers","type_hint":"instruction"}"#,
    r#"{"content":"PostgreSQL chosen over MySQL for better JSON support and pgvector extension availability","type_hint":"decision","tags":["database"]}"#,
    r#"{"content":"Team uses conventional commits format: feat/fix/docs/chore followed by scope and description","type_hint":"fact","tags":["git","process"]}"#,
];

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_import_jsonl_end_to_end(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let file = make_jsonl_file(SAMPLE_MEMORIES);

    let opts = ImportOpts::default();
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);

    let result = engine.run(&reader, file.path()).await.unwrap();

    assert_eq!(result.total, 5, "Should read 5 chunks from JSONL");
    assert_eq!(result.filtered, 0, "No chunks should be filtered (all > 50 chars)");
    assert_eq!(result.failed, 0, "No failures expected");
    assert_eq!(result.imported, 5, "All 5 memories should be imported");
    assert_eq!(result.skipped_dedup, 0, "No dedup skips on first run");

    // Verify memories exist in database.
    let list_result = store
        .list(ListFilter {
            limit: 20,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(list_result.memories.len(), 5, "5 memories should be in DB");

    // Verify auto-tags were added.
    let first = &list_result.memories[0];
    let tags = first.tags.as_ref()
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(tags.contains(&"imported"), "Should have 'imported' tag");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_import_dry_run_does_not_write(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let file = make_jsonl_file(SAMPLE_MEMORIES);

    let opts = ImportOpts {
        dry_run: true,
        ..ImportOpts::default()
    };
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);

    let result = engine.run(&reader, file.path()).await.unwrap();

    // Dry-run should report what would be imported...
    assert_eq!(result.imported, 5, "Dry run reports 5 would-be imports");

    // ...but NOT write to the database.
    let list_result = store
        .list(ListFilter {
            limit: 20,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(list_result.memories.len(), 0, "Dry run must not write to DB");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_import_dedup_on_reimport(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let file = make_jsonl_file(SAMPLE_MEMORIES);

    let opts = ImportOpts::default();
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);

    // First import: all 5 memories stored.
    let first_result = engine.run(&reader, file.path()).await.unwrap();
    assert_eq!(first_result.imported, 5);

    // Second import (same file): should dedup-skip all 5.
    let opts2 = ImportOpts::default();
    let engine2 = ImportEngine::new(store.clone(), opts2);
    let second_result = engine2.run(&reader, file.path()).await.unwrap();

    assert_eq!(second_result.imported, 0, "Re-import should import 0 (all deduped)");
    assert_eq!(second_result.skipped_dedup, 5, "All 5 should be skipped as duplicates");

    // Database should still have exactly 5 memories.
    let list_result = store
        .list(ListFilter {
            limit: 20,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(list_result.memories.len(), 5, "Still only 5 memories in DB");
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_import_noise_filter_drops_short_content(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());

    // Mix of long (signal) and short (noise) memories.
    let lines = &[
        r#"{"content":"Short"}"#,           // < 50 chars — noise
        r#"{"content":"Also too short"}"#,  // < 50 chars — noise
        r#"{"content":"User prefers Rust for backend systems because of safety guarantees"}"#, // signal
    ];
    let file = make_jsonl_file(lines);

    let opts = ImportOpts::default();
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);

    let result = engine.run(&reader, file.path()).await.unwrap();

    assert_eq!(result.total, 3);
    assert_eq!(result.filtered, 2, "2 short memories should be filtered");
    assert_eq!(result.imported, 1, "Only 1 long memory should be imported");
}

/// Debug test: verify check_existing returns correct hashes for stored content.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_dedup_check_existing_finds_stored_content(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool.clone()).await.unwrap());
    let file = make_jsonl_file(&[
        r#"{"content":"User prefers Rust for backend services due to memory safety guarantees here"}"#,
    ]);

    // Import once.
    let opts = ImportOpts::default();
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);
    let result = engine.run(&reader, file.path()).await.unwrap();
    assert_eq!(result.imported, 1, "First import should store 1 memory");

    // Verify content is in DB.
    let db_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(db_count, 1, "DB should have 1 memory after import");

    // Compute the hash of the content.
    let content = "User prefers Rust for backend services due to memory safety guarantees here";
    let hash = dedup::normalized_hash(content);

    // check_existing should find this hash.
    let found = dedup::check_existing(&pool, &[hash.clone()]).await.unwrap();
    assert!(
        found.contains(&hash),
        "check_existing should find the hash of the stored content, got: {:?}",
        found
    );
}

#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_import_with_project_and_extra_tags(pool: PgPool) {
    let store = Arc::new(PostgresMemoryStore::from_pool(pool).await.unwrap());
    let file = make_jsonl_file(&[
        r#"{"content":"User prefers Rust for backend systems and enjoys memory safety guarantees here"}"#,
    ]);

    let opts = ImportOpts {
        project: Some("my-project".to_string()),
        tags: vec!["extra-tag".to_string(), "cli-added".to_string()],
        ..ImportOpts::default()
    };
    let reader = JsonlReader;
    let engine = ImportEngine::new(store.clone(), opts);

    let result = engine.run(&reader, file.path()).await.unwrap();
    assert_eq!(result.imported, 1);

    let list_result = store
        .list(ListFilter {
            limit: 5,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(list_result.memories.len(), 1);
    let memory = &list_result.memories[0];

    let tags = memory.tags.as_ref()
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    assert!(tags.contains(&"imported"), "Should have auto 'imported' tag");
    assert!(tags.contains(&"extra-tag"), "Should have CLI-added extra-tag");
    assert!(tags.contains(&"cli-added"), "Should have CLI-added cli-added");
}
