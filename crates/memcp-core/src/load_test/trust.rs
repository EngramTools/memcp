//! Trust workload core module — correctness tracking, mock LLM provider,
//! curation cycle triggering, and audit logic for trust/security load testing.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock};

// ─── Violation Types ─────────────────────────────────────────────────────────

/// Types of correctness violations detected during trust workload execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationType {
    /// A quarantined memory ID appeared in search results.
    QuarantinedInSearchResults,
    /// A quarantined memory is missing the "suspicious" tag.
    MissingTag,
    /// A quarantined memory has wrong trust_level (should be 0.05).
    WrongTrustLevel,
    /// A quarantined memory has no trust_history in metadata.
    MissingAuditTrail,
    /// A poisoned memory was not quarantined within the expected detection window.
    UndetectedPoison,
}

/// A single correctness violation found during the trust workload run.
#[derive(Debug, Clone)]
pub struct CorrectnessViolation {
    pub violation_type: ViolationType,
    pub memory_id: String,
    pub expected: String,
    pub actual: String,
    pub timestamp: DateTime<Utc>,
}

// ─── Curation Metrics ────────────────────────────────────────────────────────

/// Timing and throughput data from curation cycles during the trust workload.
#[derive(Debug, Default)]
pub struct CurationMetrics {
    pub cycle_count: usize,
    pub total_suspicious: usize,
    pub p1_drain_ms: Vec<u64>,
    pub p2_drain_ms: Vec<u64>,
    pub normal_drain_ms: Vec<u64>,
    pub dwell_times_ms: Vec<u64>,
}

// ─── Trust Workload State ────────────────────────────────────────────────────

/// Shared state for the trust workload, tracking quarantined IDs and violations.
pub struct TrustWorkloadState {
    /// IDs of memories confirmed quarantined (via DB check).
    pub quarantined_ids: Arc<RwLock<HashSet<String>>>,
    /// All poisoned memory IDs seeded into the corpus.
    pub poisoned_ids: Arc<RwLock<HashSet<String>>>,
    /// Accumulated correctness violations found during the run.
    pub violations: Arc<Mutex<Vec<CorrectnessViolation>>>,
    /// Timing data from curation cycles.
    pub curation_metrics: Arc<Mutex<CurationMetrics>>,
}

impl TrustWorkloadState {
    /// Create a new empty trust workload state.
    pub fn new() -> Self {
        Self {
            quarantined_ids: Arc::new(RwLock::new(HashSet::new())),
            poisoned_ids: Arc::new(RwLock::new(HashSet::new())),
            violations: Arc::new(Mutex::new(Vec::new())),
            curation_metrics: Arc::new(Mutex::new(CurationMetrics::default())),
        }
    }
}

// ─── Inline Assertion: Search Results ────────────────────────────────────────

/// Check search result IDs against known quarantined IDs.
///
/// For each quarantined ID found in search results, adds a
/// `QuarantinedInSearchResults` violation. Returns the count of violations found.
pub async fn check_search_results(
    state: &TrustWorkloadState,
    result_ids: &[String],
) -> usize {
    let quarantined = state.quarantined_ids.read().await;
    let mut violation_count = 0;

    for id in result_ids {
        if quarantined.contains(id) {
            let violation = CorrectnessViolation {
                violation_type: ViolationType::QuarantinedInSearchResults,
                memory_id: id.clone(),
                expected: "excluded from search results".to_string(),
                actual: "appeared in search results".to_string(),
                timestamp: Utc::now(),
            };
            state.violations.lock().await.push(violation);
            violation_count += 1;
        }
    }

    violation_count
}

// ─── Post-Run Audit ──────────────────────────────────────────────────────────

/// Audit a single memory for quarantine consistency.
///
/// Checks:
/// - tags contains "suspicious" (MissingTag if not)
/// - trust_level == 0.05 within tolerance (WrongTrustLevel if not)
/// - metadata.trust_history exists (MissingAuditTrail if not)
///
/// Returns violations found (empty vec if memory is correctly quarantined).
pub fn audit_memory(
    memory_id: &str,
    tags: Option<&serde_json::Value>,
    trust_level: f32,
    metadata: Option<&serde_json::Value>,
) -> Vec<CorrectnessViolation> {
    let mut violations = Vec::new();

    // Check for "suspicious" tag
    let has_suspicious = tags
        .and_then(|t| t.as_array())
        .map_or(false, |arr| {
            arr.iter().any(|v| v.as_str() == Some("suspicious"))
        });

    if !has_suspicious {
        violations.push(CorrectnessViolation {
            violation_type: ViolationType::MissingTag,
            memory_id: memory_id.to_string(),
            expected: "tags contain \"suspicious\"".to_string(),
            actual: format!("tags: {:?}", tags),
            timestamp: Utc::now(),
        });
    }

    // Check trust_level == 0.05 within tolerance
    if (trust_level - 0.05).abs() > 0.001 {
        violations.push(CorrectnessViolation {
            violation_type: ViolationType::WrongTrustLevel,
            memory_id: memory_id.to_string(),
            expected: "0.05".to_string(),
            actual: format!("{:.4}", trust_level),
            timestamp: Utc::now(),
        });
    }

    // Check metadata.trust_history exists
    let has_audit_trail = metadata
        .and_then(|m| m.get("trust_history"))
        .is_some();

    if !has_audit_trail {
        violations.push(CorrectnessViolation {
            violation_type: ViolationType::MissingAuditTrail,
            memory_id: memory_id.to_string(),
            expected: "metadata contains trust_history".to_string(),
            actual: format!("metadata: {:?}", metadata),
            timestamp: Utc::now(),
        });
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── check_search_results tests ───────────────────────────────────────

    #[tokio::test]
    async fn test_check_search_results_with_quarantined_id() {
        let state = TrustWorkloadState::new();
        state
            .quarantined_ids
            .write()
            .await
            .insert("mem-001".to_string());

        let result_ids = vec!["mem-001".to_string(), "mem-002".to_string()];
        let count = check_search_results(&state, &result_ids).await;

        assert_eq!(count, 1, "Should detect 1 quarantined ID in results");
        let violations = state.violations.lock().await;
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].violation_type,
            ViolationType::QuarantinedInSearchResults
        );
        assert_eq!(violations[0].memory_id, "mem-001");
    }

    #[tokio::test]
    async fn test_check_search_results_no_quarantined() {
        let state = TrustWorkloadState::new();
        state
            .quarantined_ids
            .write()
            .await
            .insert("mem-999".to_string());

        let result_ids = vec!["mem-001".to_string(), "mem-002".to_string()];
        let count = check_search_results(&state, &result_ids).await;

        assert_eq!(count, 0, "Should detect no quarantined IDs");
        let violations = state.violations.lock().await;
        assert!(violations.is_empty());
    }

    // ── audit_memory tests ───────────────────────────────────────────────

    #[test]
    fn test_audit_memory_missing_suspicious_tag() {
        let tags = json!(["fact", "important"]);
        let metadata = json!({"trust_history": [{"from": 0.5, "to": 0.05}]});

        let violations = audit_memory("mem-001", Some(&tags), 0.05, Some(&metadata));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].violation_type, ViolationType::MissingTag);
    }

    #[test]
    fn test_audit_memory_wrong_trust_level() {
        let tags = json!(["suspicious"]);
        let metadata = json!({"trust_history": [{"from": 0.5, "to": 0.05}]});

        let violations = audit_memory("mem-001", Some(&tags), 0.5, Some(&metadata));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].violation_type, ViolationType::WrongTrustLevel);
    }

    #[test]
    fn test_audit_memory_missing_audit_trail() {
        let tags = json!(["suspicious"]);
        let metadata = json!({"some_other_key": "value"});

        let violations = audit_memory("mem-001", Some(&tags), 0.05, Some(&metadata));

        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].violation_type,
            ViolationType::MissingAuditTrail
        );
    }

    #[test]
    fn test_audit_memory_correct_state() {
        let tags = json!(["suspicious", "auto-flagged"]);
        let metadata = json!({"trust_history": [{"from": 0.5, "to": 0.05}]});

        let violations = audit_memory("mem-001", Some(&tags), 0.05, Some(&metadata));

        assert!(
            violations.is_empty(),
            "Correct quarantine state should produce no violations, got {:?}",
            violations.len()
        );
    }

    #[test]
    fn test_audit_memory_null_tags() {
        let metadata = json!({"trust_history": [{"from": 0.5, "to": 0.05}]});

        let violations = audit_memory("mem-001", None, 0.05, Some(&metadata));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].violation_type, ViolationType::MissingTag);
    }

    #[test]
    fn test_audit_memory_null_metadata() {
        let tags = json!(["suspicious"]);

        let violations = audit_memory("mem-001", Some(&tags), 0.05, None);

        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].violation_type,
            ViolationType::MissingAuditTrail
        );
    }

    #[test]
    fn test_violations_accumulated_not_fatal() {
        // Memory with ALL issues: missing tag, wrong trust, no audit trail
        let tags = json!(["fact"]);
        let metadata = json!({"other": true});

        let violations = audit_memory("mem-001", Some(&tags), 0.8, Some(&metadata));

        assert_eq!(
            violations.len(),
            3,
            "Should accumulate all 3 violations, got {}",
            violations.len()
        );

        let types: Vec<_> = violations.iter().map(|v| &v.violation_type).collect();
        assert!(types.contains(&&ViolationType::MissingTag));
        assert!(types.contains(&&ViolationType::WrongTrustLevel));
        assert!(types.contains(&&ViolationType::MissingAuditTrail));
    }
}
