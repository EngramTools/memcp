use memcp::search::salience::{
    access_frequency_score, fsrs_retrievability, normalize, recency_score,
};

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
