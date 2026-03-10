//! CSV formatter for the export pipeline.
//!
//! Produces a flat CSV with a header row followed by one data row per memory.
//! Embeddings are excluded from CSV output (too large for columnar format).
//! Optional --include-state adds stability/difficulty/reinforcement_count columns.
//!
//! Field escaping: fields containing commas, double quotes, or newlines are
//! wrapped in double quotes. Internal double quotes are doubled ("").

use std::io::Write;

use anyhow::Result;

use super::{ExportOpts, ExportableMemory};

/// Escape a single CSV field value.
///
/// Wraps in double quotes if the value contains commas, newlines, or double quotes.
/// Internal double quotes are doubled.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('\n') || s.contains('\r') || s.contains('"') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

/// Write memories as CSV — header row followed by one data row per memory.
///
/// Tags are serialized as a space-separated list within the cell.
/// Embeddings are not included (too large for columnar format).
pub fn write_csv(
    writer: &mut dyn Write,
    memories: &[ExportableMemory],
    opts: &ExportOpts,
) -> Result<()> {
    // Build header row.
    let mut headers = vec![
        "id",
        "content",
        "type_hint",
        "source",
        "tags",
        "created_at",
        "actor",
        "actor_type",
        "audience",
        "project",
    ];
    if opts.include_state {
        headers.push("stability");
        headers.push("difficulty");
        headers.push("reinforcement_count");
    }

    writeln!(writer, "{}", headers.join(","))?;

    // Write data rows.
    for mem in memories {
        let tags_str: String = mem
            .tags
            .as_ref()
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();

        let mut fields = vec![
            csv_escape(&mem.id),
            csv_escape(&mem.content),
            csv_escape(&mem.type_hint),
            csv_escape(&mem.source),
            csv_escape(&tags_str),
            csv_escape(&mem.created_at.to_rfc3339()),
            csv_escape(mem.actor.as_deref().unwrap_or("")),
            csv_escape(&mem.actor_type),
            csv_escape(&mem.audience),
            csv_escape(mem.project.as_deref().unwrap_or("")),
        ];

        if opts.include_state {
            fields.push(csv_escape(&mem.stability.unwrap_or(0.0).to_string()));
            fields.push(csv_escape(&mem.difficulty.unwrap_or(0.0).to_string()));
            fields.push(csv_escape(
                &mem.reinforcement_count.unwrap_or(0).to_string(),
            ));
        }

        writeln!(writer, "{}", fields.join(","))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_memory(id: &str, content: &str, type_hint: &str) -> ExportableMemory {
        ExportableMemory {
            id: id.to_string(),
            content: content.to_string(),
            type_hint: type_hint.to_string(),
            source: "test-source".to_string(),
            tags: Some(serde_json::Value::Array(vec![
                serde_json::Value::String("tag1".to_string()),
                serde_json::Value::String("tag2".to_string()),
            ])),
            created_at: Utc::now(),
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            project: None,
            event_time: None,
            event_time_precision: None,
            stability: Some(2.5),
            difficulty: Some(5.0),
            reinforcement_count: Some(3),
            last_reinforced_at: None,
            embedding: None,
            embedding_model: None,
        }
    }

    #[test]
    fn test_csv_output_headers() {
        let memories = vec![make_memory("id1", "content here", "fact")];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_csv(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();

        // Header row + 1 data row.
        assert_eq!(lines.len(), 2);

        // Header must contain expected columns.
        assert!(lines[0].contains("id"), "header must contain 'id'");
        assert!(
            lines[0].contains("content"),
            "header must contain 'content'"
        );
        assert!(
            lines[0].contains("type_hint"),
            "header must contain 'type_hint'"
        );
        assert!(lines[0].contains("source"), "header must contain 'source'");
        assert!(lines[0].contains("tags"), "header must contain 'tags'");
        assert!(
            lines[0].contains("created_at"),
            "header must contain 'created_at'"
        );
    }

    #[test]
    fn test_csv_data_rows_match_count() {
        let memories = vec![
            make_memory("id1", "first memory content", "fact"),
            make_memory("id2", "second memory content", "preference"),
            make_memory("id3", "third memory content", "instruction"),
        ];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_csv(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();

        // 1 header + 3 data rows.
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_csv_escapes_commas_in_content() {
        let memories = vec![make_memory("id1", "content with, a comma inside", "fact")];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_csv(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();
        // Data row must have the content wrapped in double quotes.
        assert!(lines[1].contains("\"content with, a comma inside\""));
    }

    #[test]
    fn test_csv_include_state_columns() {
        let memories = vec![make_memory("id1", "some content here", "fact")];
        let opts = ExportOpts {
            include_state: true,
            ..ExportOpts::default()
        };

        let mut buf = Vec::new();
        write_csv(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();

        assert!(
            lines[0].contains("stability"),
            "header must contain 'stability'"
        );
        assert!(
            lines[0].contains("difficulty"),
            "header must contain 'difficulty'"
        );
        assert!(
            lines[0].contains("reinforcement_count"),
            "header must contain 'reinforcement_count'"
        );
    }

    #[test]
    fn test_csv_empty_memories() {
        let opts = ExportOpts::default();
        let mut buf = Vec::new();
        write_csv(&mut buf, &[], &opts).unwrap();

        // Even with no memories, header row is written.
        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();
        assert_eq!(
            lines.len(),
            1,
            "should have header row even with no memories"
        );
    }
}
