//! Unit stub for `parse_ingest_stream` (Phase 24.5 / D-21).
//!
//! `parse_ingest_stream` must auto-detect the wire format from the first non-whitespace
//! character:
//!   - input starting with `[` → JSON-array parse (one Vec<IngestMessage>)
//!   - input starting with `{` → JSONL stream parse (one message per line)
//! Both routes must yield an equivalent message Vec when fed semantically identical
//! payloads.
//!
//! Stub is `#[ignore]`d until Plan 24.5-04 lands `parse_ingest_stream`.

// TODO(24.5-04): uncomment once parse_ingest_stream is introduced.
// use memcp::transport::api::ingest::parse_ingest_stream;

/// D-21: `parse_ingest_stream` auto-detects `[` (array) vs `{` (JSONL).
#[ignore = "24.5-04 impl pending"]
#[test]
fn test_parse_stream_autodetect() {
    unimplemented!("24.5-04");
}
