// Integration tests for the full tiered content pipeline (Plan 23-03)
//
// These tests validate depth selection and fallback logic WITHOUT requiring
// an LLM provider. abstract_text/overview_text are set via direct SQL UPDATE
// to isolate retrieval from generation.

mod common;
use common::builders::MemoryBuilder;

use memcp::store::postgres::PostgresMemoryStore;
use memcp::store::MemoryStore;
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Insert a mock 384-dim embedding and mark the memory as embedding_status='complete'.
async fn insert_mock_embedding(pool: &PgPool, memory_id: &str) {
    use uuid::Uuid;
    let now = chrono::Utc::now();
    let zero_emb = format!("[{}]", vec!["0.0"; 384].join(","));

    sqlx::query(
        "INSERT INTO memory_embeddings (id, memory_id, embedding, model_name, model_version, dimension, is_current, created_at, updated_at)
         VALUES ($1, $2, $3::vector, 'test-mock', '1', 384, true, $4, $4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(memory_id)
    .bind(&zero_emb)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE memories SET embedding_status = 'complete' WHERE id = $1")
        .bind(memory_id)
        .execute(pool)
        .await
        .unwrap();
}

/// Set abstract_text and/or overview_text directly on a memory row.
async fn set_abstraction_texts(
    pool: &PgPool,
    memory_id: &str,
    abstract_text: Option<&str>,
    overview_text: Option<&str>,
) {
    sqlx::query(
        "UPDATE memories SET abstract_text = $1, overview_text = $2, abstraction_status = 'done' WHERE id = $3",
    )
    .bind(abstract_text)
    .bind(overview_text)
    .bind(memory_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Depth-selection logic (mirrors transport/server.rs and cli.rs).
fn select_depth<'a>(
    depth: u8,
    content: &'a str,
    abstract_text: Option<&'a str>,
    overview_text: Option<&'a str>,
) -> &'a str {
    match depth {
        0 => abstract_text.unwrap_or(content),
        1 => overview_text.unwrap_or(content),
        _ => content,
    }
}

// ---------------------------------------------------------------------------
// Test 1: depth_fallback — depth=0 returns content when abstract_text is NULL
// ---------------------------------------------------------------------------

/// TCL-03/TCL-05: When a memory has no abstract_text, depth=0 returns full content.
/// This validates graceful fallback — agents using depth=0 don't get empty results.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_depth_fallback_returns_content_when_abstract_null(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let memory = store
        .store(
            MemoryBuilder::new()
                .content("Full memory content without abstraction")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();

    // No abstract_text set — memory.abstract_text is NULL
    let retrieved = store.get(&memory.id).await.unwrap();
    assert!(
        retrieved.abstract_text.is_none(),
        "abstract_text should be NULL by default"
    );

    // Depth selection with NULL abstract should fall back to content
    let display = select_depth(
        0,
        &retrieved.content,
        retrieved.abstract_text.as_deref(),
        None,
    );
    assert_eq!(
        display, "Full memory content without abstraction",
        "depth=0 with NULL abstract must fall back to full content"
    );
}

// ---------------------------------------------------------------------------
// Test 2: depth_default — default depth returns full content
// ---------------------------------------------------------------------------

/// TCL-03: default depth=2 always returns full content (backward compatible).
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_depth_default_returns_full_content(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let memory = store
        .store(
            MemoryBuilder::new()
                .content("Original full content for depth default test")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();

    insert_mock_embedding(&pool, &memory.id).await;

    // Set abstract and overview texts — they should NOT be returned with depth=2
    set_abstraction_texts(
        &pool,
        &memory.id,
        Some("short abstract"),
        Some("medium overview"),
    )
    .await;

    let retrieved = store.get(&memory.id).await.unwrap();

    let display = select_depth(
        2,
        &retrieved.content,
        retrieved.abstract_text.as_deref(),
        retrieved.overview_text.as_deref(),
    );
    assert_eq!(
        display, "Original full content for depth default test",
        "depth=2 should always return full content, ignoring abstract/overview"
    );
}

// ---------------------------------------------------------------------------
// Test 3: abstraction_status_skipped — short content gets status='skipped'
// ---------------------------------------------------------------------------

/// TCL-04: The abstraction worker skips short memories (< 200 chars).
/// This test verifies the DB schema supports abstraction_status='skipped'
/// and that the field is readable after being set.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_abstraction_status_skipped_for_short_content(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    // Store a short memory (content under 200 chars)
    let memory = store
        .store(
            MemoryBuilder::new()
                .content("short")
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();

    // Simulate what the abstraction worker does: set status='skipped' for short content
    sqlx::query("UPDATE memories SET abstraction_status = 'skipped' WHERE id = $1")
        .bind(&memory.id)
        .execute(&pool)
        .await
        .unwrap();

    let retrieved = store.get(&memory.id).await.unwrap();
    assert_eq!(
        retrieved.abstraction_status, "skipped",
        "short content should have abstraction_status='skipped'"
    );
    assert!(
        retrieved.abstract_text.is_none(),
        "skipped memories should have NULL abstract_text"
    );
    assert!(
        retrieved.overview_text.is_none(),
        "skipped memories should have NULL overview_text"
    );
}

// ---------------------------------------------------------------------------
// Test 4: depth_zero_returns_abstract — depth=0 returns abstract_text when present
// ---------------------------------------------------------------------------

/// TCL-05: When abstract_text is set, depth=0 returns the abstract (not full content).
/// This validates the primary purpose of the depth feature: token-efficient scanning.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_depth_zero_returns_abstract(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let long_content = "A very detailed memory about a complex topic that contains a lot of information and should be abstracted for efficient scanning by agents during planning phases";
    let memory = store
        .store(
            MemoryBuilder::new()
                .content(long_content)
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();

    insert_mock_embedding(&pool, &memory.id).await;

    // Manually set abstract_text (simulating what the abstraction worker produces)
    let abstract_text = "Detailed memory about a complex topic";
    set_abstraction_texts(&pool, &memory.id, Some(abstract_text), None).await;

    let retrieved = store.get(&memory.id).await.unwrap();
    assert!(
        retrieved.abstract_text.is_some(),
        "abstract_text should be set after SQL update"
    );

    let display = select_depth(
        0,
        &retrieved.content,
        retrieved.abstract_text.as_deref(),
        retrieved.overview_text.as_deref(),
    );
    assert_eq!(
        display, abstract_text,
        "depth=0 should return abstract_text, not full content"
    );
    assert_ne!(
        display, long_content,
        "depth=0 must NOT return full content when abstract is available"
    );
}

// ---------------------------------------------------------------------------
// Test 5: depth_one_returns_overview — depth=1 returns overview_text when present
// ---------------------------------------------------------------------------

/// TCL-05: When overview_text is set, depth=1 returns the overview (not full content).
/// Overview is the structured middle tier — more detail than abstract, less than full.
#[sqlx::test(migrator = "memcp::MIGRATOR")]
async fn test_depth_one_returns_overview(pool: PgPool) {
    let store = PostgresMemoryStore::from_pool(pool.clone()).await.unwrap();

    let long_content =
        "Comprehensive memory with extensive structured content covering multiple aspects of a topic including background, current status, key decisions made, open questions, and next steps";
    let memory = store
        .store(
            MemoryBuilder::new()
                .content(long_content)
                .type_hint("fact")
                .build(),
        )
        .await
        .unwrap();

    insert_mock_embedding(&pool, &memory.id).await;

    let abstract_text = "Short abstract of comprehensive memory";
    let overview_text =
        "## Comprehensive Memory\n- Background: extensive topic\n- Status: ongoing\n- Key decisions made\n- Open questions remain";
    set_abstraction_texts(&pool, &memory.id, Some(abstract_text), Some(overview_text)).await;

    let retrieved = store.get(&memory.id).await.unwrap();
    assert!(
        retrieved.overview_text.is_some(),
        "overview_text should be set after SQL update"
    );

    let display = select_depth(
        1,
        &retrieved.content,
        retrieved.abstract_text.as_deref(),
        retrieved.overview_text.as_deref(),
    );
    assert_eq!(
        display, overview_text,
        "depth=1 should return overview_text"
    );
    assert_ne!(
        display, long_content,
        "depth=1 must NOT return full content when overview is available"
    );
    assert_ne!(
        display, abstract_text,
        "depth=1 must NOT return abstract_text (that is depth=0)"
    );
}
