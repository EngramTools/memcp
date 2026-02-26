use memcp::config::GcConfig;
use memcp::gc::GcResult;

#[test]
fn test_gc_config_defaults() {
    let config = GcConfig::default();
    assert!(config.enabled);
    assert_eq!(config.salience_threshold, 0.3);
    assert_eq!(config.min_age_days, 30);
    assert_eq!(config.min_memory_floor, 100);
    assert_eq!(config.gc_interval_secs, 3600);
    assert_eq!(config.hard_purge_grace_days, 30);
}

#[test]
fn test_gc_result_skipped() {
    let result = GcResult::skipped("below floor");
    assert_eq!(result.pruned_count, 0);
    assert_eq!(result.expired_count, 0);
    assert_eq!(result.hard_purged_count, 0);
    assert_eq!(result.skipped_reason, Some("below floor".to_string()));
    assert!(result.candidates.is_empty());
}
