//! Tests for import history tracking and discover UX.
//!
//! These tests exercise the file-based JSONL history store:
//! append_record, find_record (latest), and history file creation.

use memcp::import::history::{append_record_to, find_record_in, load_history_from, ImportHistoryRecord};
use chrono::Utc;
use tempfile::TempDir;

/// Helper: create a test record.
fn make_record(source_type: &str, path: &str, count: usize) -> ImportHistoryRecord {
    ImportHistoryRecord {
        source_type: source_type.to_string(),
        path: path.to_string(),
        count,
        timestamp: Utc::now(),
    }
}

/// Appending a record and calling find_record_in returns the appended record.
#[test]
fn test_history_append_and_find() {
    let dir = TempDir::new().unwrap();
    let history_file = dir.path().join("history.jsonl");

    let record = make_record("openclaw", "/home/user/.openclaw/main.sqlite", 100);
    append_record_to(&history_file, &record).expect("append_record should succeed");

    let found = find_record_in(&history_file, "openclaw");
    assert!(found.is_some(), "find_record should return Some for known source");
    let found = found.unwrap();
    assert_eq!(found.source_type, "openclaw");
    assert_eq!(found.count, 100);
}

/// find_record_in returns None for a source type that has no history.
#[test]
fn test_history_find_returns_none_for_unknown() {
    let dir = TempDir::new().unwrap();
    let history_file = dir.path().join("history.jsonl");

    // No records appended — should return None.
    let found = find_record_in(&history_file, "claude-code");
    assert!(found.is_none(), "find_record should return None for unknown source");
}

/// When two records exist for the same source, find_record_in returns the most recent one.
#[test]
fn test_history_multiple_records_returns_latest() {
    let dir = TempDir::new().unwrap();
    let history_file = dir.path().join("history.jsonl");

    let first = ImportHistoryRecord {
        source_type: "openclaw".to_string(),
        path: "/first".to_string(),
        count: 100,
        timestamp: chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    };
    let second = ImportHistoryRecord {
        source_type: "openclaw".to_string(),
        path: "/second".to_string(),
        count: 200,
        timestamp: chrono::DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    };

    append_record_to(&history_file, &first).expect("append first record");
    append_record_to(&history_file, &second).expect("append second record");

    let found = find_record_in(&history_file, "openclaw");
    assert!(found.is_some());
    let found = found.unwrap();
    // Should return second (more recent) record.
    assert_eq!(found.count, 200, "find_record should return the most recent record");
    assert_eq!(found.path, "/second");
}

/// After the first append_record_to call, the history.jsonl file must exist.
#[test]
fn test_history_file_created_on_first_append() {
    let dir = TempDir::new().unwrap();
    let history_file = dir.path().join("history.jsonl");

    assert!(!history_file.exists(), "history file should not exist before first append");

    let record = make_record("chatgpt", "/exports/chatgpt.zip", 42);
    append_record_to(&history_file, &record).expect("append_record should succeed");

    assert!(history_file.exists(), "history file should be created after first append");

    // Verify we can load the record back.
    let records = load_history_from(&history_file);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_type, "chatgpt");
    assert_eq!(records[0].count, 42);
}
