//! Unit test for `parse_ingest_stream` (Phase 24.5 / D-21).
//!
//! `parse_ingest_stream` auto-detects the wire format from the first non-whitespace
//! character:
//!   - input starting with `[` → JSON-array parse (one Vec<IngestMessage>)
//!   - input starting with `{` → JSONL stream parse (one message per line)
//! Both routes must yield an equivalent message Vec when fed semantically identical
//! payloads.

use memcp::transport::api::ingest::parse_ingest_stream;
use memcp::transport::api::types::IngestMessage;
use serde_json::json;

fn make_msg(role: &str, content: &str) -> IngestMessage {
    IngestMessage {
        role: role.to_string(),
        content: content.to_string(),
        timestamp: None,
        idempotency_key: None,
        reply_to_id: None,
    }
}

/// D-21: `parse_ingest_stream` auto-detects `[` (array) vs `{` (JSONL).
#[test]
fn test_parse_stream_autodetect() {
    let m1 = make_msg("user", "hello");
    let m2 = make_msg("assistant", "hi there");

    // JSON array form.
    let array = json!([m1, m2]).to_string();
    let parsed_array = parse_ingest_stream(&array).expect("array parse");
    assert_eq!(parsed_array.len(), 2, "array should yield 2 messages");
    assert_eq!(parsed_array[0].role, "user");
    assert_eq!(parsed_array[0].content, "hello");
    assert_eq!(parsed_array[1].role, "assistant");
    assert_eq!(parsed_array[1].content, "hi there");

    // JSONL form: two objects separated by newline.
    let jsonl = format!("{}\n{}", json!(m1), json!(m2));
    let parsed_jsonl = parse_ingest_stream(&jsonl).expect("jsonl parse");
    assert_eq!(parsed_jsonl.len(), 2, "jsonl should yield 2 messages");
    assert_eq!(parsed_jsonl[0].role, "user");
    assert_eq!(parsed_jsonl[1].role, "assistant");

    // Leading whitespace is skipped on either form.
    let padded_array = format!("   \n\t{}", array);
    assert_eq!(
        parse_ingest_stream(&padded_array).expect("padded").len(),
        2,
        "leading whitespace should not change detection"
    );

    // Empty / whitespace-only input yields an empty vec.
    let empty = parse_ingest_stream("   \n\t").expect("empty parse");
    assert_eq!(empty.len(), 0, "whitespace-only => empty vec");
    let truly_empty = parse_ingest_stream("").expect("truly empty");
    assert_eq!(truly_empty.len(), 0, "empty input => empty vec");
}
