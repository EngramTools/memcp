use chrono::{DateTime, Duration, TimeZone, Utc};
use memcp::query_intelligence::temporal::parse_temporal_hint;
use memcp::query_intelligence::TimeRange;
use memcp::store::Memory;

// ---------------------------------------------------------------------------
// Bi-temporal helpers
// ---------------------------------------------------------------------------

/// Builds a minimal Memory for bi-temporal tests.
/// Only created_at and event_time matter for the boost logic.
fn make_memory(created_at: DateTime<Utc>, event_time: Option<DateTime<Utc>>) -> Memory {
    Memory {
        id: "test-id".to_string(),
        content: "test content".to_string(),
        type_hint: "fact".to_string(),
        source: "test".to_string(),
        tags: None,
        created_at,
        updated_at: created_at,
        last_accessed_at: None,
        access_count: 0,
        embedding_status: "pending".to_string(),
        extracted_entities: None,
        extracted_facts: None,
        extraction_status: "pending".to_string(),
        is_consolidated_original: false,
        consolidated_into: None,
        actor: None,
        actor_type: "agent".to_string(),
        audience: "global".to_string(),
        event_time,
        event_time_precision: None,
        project: None,
        trust_level: 0.5,
        session_id: None,
        agent_role: None,
        write_path: None,
        metadata: serde_json::json!({}),
        abstract_text: None,
        overview_text: None,
        abstraction_status: "skipped".to_string(),
        knowledge_tier: "explicit".to_string(),
        source_ids: None,
        reply_to_id: None,
    }
}

/// Replicates the bi-temporal selection used in the server.rs temporal boost:
///   `event_time.unwrap_or(created_at)`
/// Returns true when the effective timestamp falls within the given range.
fn bi_temporal_in_range(memory: &Memory, range: &TimeRange) -> bool {
    let t = memory.event_time.unwrap_or(memory.created_at);
    match (range.after, range.before) {
        (Some(after), Some(before)) => t >= after && t <= before,
        (Some(after), None) => t >= after,
        (None, Some(before)) => t <= before,
        (None, None) => false,
    }
}

fn fixed_now() -> DateTime<Utc> {
    // 2024-03-15 12:00:00 UTC
    Utc.with_ymd_and_hms(2024, 3, 15, 12, 0, 0).unwrap()
}

#[test]
fn test_no_match_returns_none() {
    assert!(parse_temporal_hint("find my API keys", fixed_now()).is_none());
}

#[test]
fn test_yesterday() {
    let now = fixed_now();
    let result = parse_temporal_hint("what did I do yesterday", now).unwrap();
    // yesterday = 2024-03-14
    assert_eq!(
        result.after.unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 14, 0, 0, 0).unwrap()
    );
    assert_eq!(
        result.before.unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 14, 23, 59, 59).unwrap()
    );
}

#[test]
fn test_today() {
    let now = fixed_now();
    let result = parse_temporal_hint("notes from today", now).unwrap();
    assert_eq!(
        result.after.unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 15, 0, 0, 0).unwrap()
    );
    assert!(result.before.is_none());
}

#[test]
fn test_last_week() {
    let now = fixed_now();
    let result = parse_temporal_hint("what happened last week", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(7));
    assert!(result.before.is_none());
}

#[test]
fn test_past_month() {
    let now = fixed_now();
    let result = parse_temporal_hint("memories from the past month", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(30));
}

#[test]
fn test_last_year() {
    let now = fixed_now();
    let result = parse_temporal_hint("notes from last year", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(365));
}

#[test]
fn test_a_few_days_ago() {
    let now = fixed_now();
    let result = parse_temporal_hint("I read that a few days ago", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(5));
    assert_eq!(result.before.unwrap(), now - Duration::days(1));
}

#[test]
fn test_a_few_weeks_ago() {
    let now = fixed_now();
    let result = parse_temporal_hint("happened a few weeks ago", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(28));
    assert_eq!(result.before.unwrap(), now - Duration::days(7));
}

#[test]
fn test_a_few_months_ago() {
    let now = fixed_now();
    let result = parse_temporal_hint("it was a few months ago", now).unwrap();
    assert_eq!(result.after.unwrap(), now - Duration::days(90));
    assert_eq!(result.before.unwrap(), now - Duration::days(30));
}

#[test]
fn test_after_date() {
    let now = fixed_now();
    let result = parse_temporal_hint("entries after 2024-01-01", now).unwrap();
    assert_eq!(
        result.after.unwrap(),
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    );
    assert!(result.before.is_none());
}

#[test]
fn test_before_date() {
    let now = fixed_now();
    let result = parse_temporal_hint("notes before 2024-02-28", now).unwrap();
    assert!(result.after.is_none());
    assert_eq!(
        result.before.unwrap(),
        Utc.with_ymd_and_hms(2024, 2, 28, 23, 59, 59).unwrap()
    );
}

#[test]
fn test_between_months() {
    let now = fixed_now();
    let result = parse_temporal_hint("between January and March", now).unwrap();
    assert_eq!(
        result.after.unwrap(),
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    );
    // End of March = April 1 00:00:00 - 1 second = March 31 23:59:59
    assert_eq!(
        result.before.unwrap(),
        Utc.with_ymd_and_hms(2024, 3, 31, 23, 59, 59).unwrap()
    );
}

// ---------------------------------------------------------------------------
// Bi-temporal boost tests
// ---------------------------------------------------------------------------

/// Test 1: event_time present and in range — should be boosted.
/// Memory stored 2026-03-03, event_time 2019-06-01, range 2019-01-01..2019-12-31.
/// event_time is in range → boost applies even though created_at is not in range.
#[test]
fn test_bitemporal_event_time_in_range_gets_boost() {
    let created_at = Utc.with_ymd_and_hms(2026, 3, 3, 0, 0, 0).unwrap();
    let event_time = Some(Utc.with_ymd_and_hms(2019, 6, 1, 0, 0, 0).unwrap());
    let memory = make_memory(created_at, event_time);

    let range = TimeRange {
        after: Some(Utc.with_ymd_and_hms(2019, 1, 1, 0, 0, 0).unwrap()),
        before: Some(Utc.with_ymd_and_hms(2019, 12, 31, 23, 59, 59).unwrap()),
    };

    assert!(
        bi_temporal_in_range(&memory, &range),
        "Memory with event_time in 2019 should be boosted for '2019 memories' query"
    );
}

/// Test 2: event_time absent — fallback to created_at.
/// Memory with no event_time but created_at 2019-06-01, range 2019-01-01..2019-12-31.
/// Falls back to created_at which is in range → boost applies.
#[test]
fn test_bitemporal_no_event_time_falls_back_to_created_at() {
    let created_at = Utc.with_ymd_and_hms(2019, 6, 1, 0, 0, 0).unwrap();
    let memory = make_memory(created_at, None);

    let range = TimeRange {
        after: Some(Utc.with_ymd_and_hms(2019, 1, 1, 0, 0, 0).unwrap()),
        before: Some(Utc.with_ymd_and_hms(2019, 12, 31, 23, 59, 59).unwrap()),
    };

    assert!(
        bi_temporal_in_range(&memory, &range),
        "Memory without event_time should fall back to created_at for temporal boost"
    );
}

/// Test 3: event_time present but out of range — no boost even though created_at is in range.
/// Memory with event_time 2020-01-01 (out of range) and created_at 2019-06-01 (in range).
/// event_time takes precedence → no boost.
#[test]
fn test_bitemporal_event_time_out_of_range_no_boost() {
    let created_at = Utc.with_ymd_and_hms(2019, 6, 1, 0, 0, 0).unwrap();
    let event_time = Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap());
    let memory = make_memory(created_at, event_time);

    let range = TimeRange {
        after: Some(Utc.with_ymd_and_hms(2019, 1, 1, 0, 0, 0).unwrap()),
        before: Some(Utc.with_ymd_and_hms(2019, 12, 31, 23, 59, 59).unwrap()),
    };

    assert!(
        !bi_temporal_in_range(&memory, &range),
        "event_time takes precedence over created_at — out-of-range event_time means no boost"
    );
}
