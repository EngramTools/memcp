use chrono::{DateTime, Duration, TimeZone, Utc};
use memcp::query_intelligence::temporal::parse_temporal_hint;

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
