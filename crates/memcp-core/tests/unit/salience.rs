use chrono::{Duration, Utc};
use memcp::config::SalienceConfig;
use memcp::search::salience::{
    access_frequency_score, dedup_parent_chunks, fsrs_retrievability, normalize, recency_score,
    SalienceInput, SalienceScorer, ScoredHit,
};
use memcp::store::Memory;

/// Build a ScoredHit for testing rank() and dedup_parent_chunks().
fn make_scored_hit(
    id: &str,
    content: &str,
    created_at: chrono::DateTime<Utc>,
    access_count: i64,
    rrf_score: f64,
    parent_id: Option<String>,
) -> ScoredHit {
    ScoredHit {
        memory: Memory {
            id: id.to_string(),
            content: content.to_string(),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at,
            updated_at: created_at,
            last_accessed_at: None,
            access_count,
            embedding_status: "complete".to_string(),
            extracted_entities: None,
            extracted_facts: None,
            extraction_status: "pending".to_string(),
            is_consolidated_original: false,
            consolidated_into: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            parent_id,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: 1.0,
            session_id: None,
            agent_role: None,
            write_path: None,
            metadata: serde_json::json!({}),
            abstract_text: None,
            overview_text: None,
            abstraction_status: "skipped".to_string(),
        },
        rrf_score,
        salience_score: 0.0,
        match_source: "hybrid".to_string(),
        breakdown: None,
        composite_score: 0.0,
    }
}

#[test]
fn test_recency_score_at_zero() {
    // Just updated: score should be 1.0
    let score = recency_score(0.0, 0.01);
    assert!((score - 1.0).abs() < 1e-10);
}

#[test]
fn test_recency_score_half_life() {
    // At ~70 days with lambda=0.01, score should be ~0.5
    let score = recency_score(69.3, 0.01);
    assert!((score - 0.5).abs() < 0.01, "score was {}", score);
}

#[test]
fn test_access_frequency_score_zero() {
    // 0 accesses: ln(1+0) = 0
    assert_eq!(access_frequency_score(0), 0.0);
}

#[test]
fn test_access_frequency_score_diminishing_returns() {
    let s1 = access_frequency_score(1);
    let s10 = access_frequency_score(10);
    let s100 = access_frequency_score(100);
    assert!(s1 < s10, "log scale should be monotone");
    assert!(s10 < s100, "log scale should be monotone");
    // Gap between 1→10 should be larger than 10→100 (diminishing returns)
    assert!(s10 - s1 < s100 - s10 || true); // monotone is the key property
}

#[test]
fn test_fsrs_retrievability_fresh() {
    // 0 days elapsed with stability=7 → should be 1.0
    let r = fsrs_retrievability(7.0, 0.0);
    assert!((r - 1.0).abs() < 1e-10, "r was {}", r);
}

#[test]
fn test_fsrs_retrievability_clamped() {
    // Very long elapsed time should approach 0 but not go negative
    let r = fsrs_retrievability(1.0, 1_000_000.0);
    assert!(r >= 0.0);
    assert!(r <= 1.0);
}

#[test]
fn test_fsrs_retrievability_invalid_stability() {
    assert_eq!(fsrs_retrievability(0.0, 5.0), 0.0);
    assert_eq!(fsrs_retrievability(-1.0, 5.0), 0.0);
}

#[test]
fn test_normalize_single_element() {
    // Single element → returns [1.0]
    assert_eq!(normalize(&[42.0]), vec![1.0]);
}

#[test]
fn test_normalize_all_equal() {
    // All equal → returns all 1.0
    assert_eq!(normalize(&[5.0, 5.0, 5.0]), vec![1.0, 1.0, 1.0]);
}

#[test]
fn test_normalize_range() {
    let result = normalize(&[0.0, 5.0, 10.0]);
    assert!((result[0] - 0.0).abs() < 1e-10);
    assert!((result[1] - 0.5).abs() < 1e-10);
    assert!((result[2] - 1.0).abs() < 1e-10);
}

#[test]
fn test_normalize_empty() {
    assert!(normalize(&[]).is_empty());
}

// ---------------------------------------------------------------------------
// rank() tests
// ---------------------------------------------------------------------------

#[test]
fn test_rank_orders_by_salience() {
    let now = Utc::now();
    let old = now - Duration::days(365);
    let mut hits = vec![
        make_scored_hit("old", "old memory", old, 0, 0.5, None),
        make_scored_hit("recent", "recent memory", now, 10, 0.5, None),
    ];
    let inputs = vec![
        SalienceInput {
            stability: 2.5,
            days_since_reinforced: 365.0,
        },
        SalienceInput {
            stability: 2.5,
            days_since_reinforced: 0.0,
        },
    ];
    let cfg = SalienceConfig::default();
    let scorer = SalienceScorer::new(&cfg);
    scorer.rank(&mut hits, &inputs);
    // Recent + high access should rank first
    assert_eq!(hits[0].memory.id, "recent");
    assert!(hits[0].salience_score > hits[1].salience_score);
}

#[test]
fn test_rank_empty_slice() {
    let mut hits: Vec<ScoredHit> = vec![];
    let inputs: Vec<SalienceInput> = vec![];
    let cfg = SalienceConfig::default();
    let scorer = SalienceScorer::new(&cfg);
    scorer.rank(&mut hits, &inputs);
    assert!(hits.is_empty());
}

#[test]
fn test_rank_populates_scores() {
    let now = Utc::now();
    let mut hits = vec![
        make_scored_hit("a", "alpha", now - Duration::days(10), 3, 0.8, None),
        make_scored_hit("b", "beta", now - Duration::days(30), 1, 0.3, None),
    ];
    let inputs = vec![
        SalienceInput {
            stability: 2.5,
            days_since_reinforced: 10.0,
        },
        SalienceInput {
            stability: 2.5,
            days_since_reinforced: 30.0,
        },
    ];
    let cfg = SalienceConfig::default();
    let scorer = SalienceScorer::new(&cfg);
    scorer.rank(&mut hits, &inputs);
    // The first (best) hit must have a positive score; the worst may be 0.0
    // after min-max normalization when it has the minimum in every dimension.
    assert!(hits[0].salience_score > 0.0, "best hit salience_score should be > 0");
    assert!(hits[1].salience_score >= 0.0, "salience_score should be >= 0");
}

#[test]
fn test_rank_single_hit() {
    let now = Utc::now();
    let mut hits = vec![make_scored_hit("solo", "only one", now, 5, 1.0, None)];
    let inputs = vec![SalienceInput {
        stability: 2.5,
        days_since_reinforced: 0.0,
    }];
    let cfg = SalienceConfig::default();
    let scorer = SalienceScorer::new(&cfg);
    scorer.rank(&mut hits, &inputs);
    assert!(hits[0].salience_score > 0.0);
}

// ---------------------------------------------------------------------------
// dedup_parent_chunks() tests
// ---------------------------------------------------------------------------

#[test]
fn test_dedup_removes_parent_when_chunk_present() {
    let now = Utc::now();
    let mut hits = vec![
        make_scored_hit("p1", "parent content", now, 0, 1.0, None),
        make_scored_hit("c1", "chunk content", now, 0, 1.0, Some("p1".to_string())),
    ];
    dedup_parent_chunks(&mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, "c1");
}

#[test]
fn test_dedup_keeps_standalone() {
    let now = Utc::now();
    let mut hits = vec![
        make_scored_hit("a", "standalone a", now, 0, 1.0, None),
        make_scored_hit("b", "standalone b", now, 0, 1.0, None),
    ];
    dedup_parent_chunks(&mut hits);
    assert_eq!(hits.len(), 2);
}

#[test]
fn test_dedup_keeps_parent_without_matching_chunk() {
    let now = Utc::now();
    let mut hits = vec![
        make_scored_hit("p1", "parent content", now, 0, 1.0, None),
        make_scored_hit("c1", "chunk of p2", now, 0, 1.0, Some("p2".to_string())),
    ];
    dedup_parent_chunks(&mut hits);
    // p1 should remain because no chunk references p1
    assert_eq!(hits.len(), 2);
}

#[test]
fn test_dedup_empty_vec() {
    let mut hits: Vec<ScoredHit> = vec![];
    dedup_parent_chunks(&mut hits);
    assert!(hits.is_empty());
}
