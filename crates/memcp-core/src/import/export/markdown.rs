//! Markdown formatter for the export pipeline.
//!
//! Produces a human-readable archive grouped by type_hint.
//! Not intended for re-import — use JSONL for round-trip fidelity.
//!
//! Format:
//!   # Memory Archive
//!   ## \<type_hint\>
//!   > content
//!   > *source: X | tags: a, b | created: 2024-01-01*
//!
//!   ---

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::Result;

use super::{ExportOpts, ExportableMemory};

/// Write memories as a grouped Markdown archive.
///
/// Memories are grouped by type_hint, with each group under a `##` heading.
/// Content appears as a blockquote, metadata as an italicized line below.
/// This format is human-readable but not machine-importable.
pub fn write_markdown(
    writer: &mut dyn Write,
    memories: &[ExportableMemory],
    opts: &ExportOpts,
) -> Result<()> {
    writeln!(writer, "# Memory Archive")?;
    writeln!(writer)?;

    if memories.is_empty() {
        writeln!(writer, "*No memories exported.*")?;
        return Ok(());
    }

    // Group memories by type_hint, preserving insertion order via BTreeMap (sorted).
    let mut groups: BTreeMap<String, Vec<&ExportableMemory>> = BTreeMap::new();
    for mem in memories {
        groups.entry(mem.type_hint.clone()).or_default().push(mem);
    }

    let group_count = groups.len();
    for (i, (type_hint, group_memories)) in groups.iter().enumerate() {
        writeln!(writer, "## {}", type_hint)?;
        writeln!(writer)?;

        for mem in group_memories {
            // Content as blockquote — handle multi-line content.
            for line in mem.content.lines() {
                writeln!(writer, "> {}", line)?;
            }
            writeln!(writer)?;

            // Metadata line.
            let tags_str: String = mem
                .tags
                .as_ref()
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();

            let mut meta_parts = vec![format!("source: {}", mem.source)];
            if !tags_str.is_empty() {
                meta_parts.push(format!("tags: {}", tags_str));
            }
            meta_parts.push(format!("created: {}", mem.created_at.format("%Y-%m-%d")));
            if let Some(ref project) = mem.project {
                meta_parts.push(format!("project: {}", project));
            }

            if opts.include_state {
                if let Some(stability) = mem.stability {
                    meta_parts.push(format!("stability: {:.2}", stability));
                }
            }

            writeln!(writer, "*{}*", meta_parts.join(" | "))?;
            writeln!(writer)?;
            writeln!(writer, "---")?;
            writeln!(writer)?;
        }

        // Extra spacing between groups (except after last group).
        if i < group_count - 1 {
            writeln!(writer)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_memory(content: &str, type_hint: &str) -> ExportableMemory {
        ExportableMemory {
            id: "test-id".to_string(),
            content: content.to_string(),
            type_hint: type_hint.to_string(),
            source: "test-source".to_string(),
            tags: Some(serde_json::Value::Array(vec![
                serde_json::Value::String("rust".to_string()),
                serde_json::Value::String("backend".to_string()),
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
            reinforcement_count: Some(0),
            last_reinforced_at: None,
            embedding: None,
            embedding_model: None,
        }
    }

    #[test]
    fn test_markdown_output_contains_type_hint_header() {
        let memories = vec![
            make_memory("Rust is preferred for backend services", "fact"),
            make_memory("Use async/await for IO-bound tasks", "instruction"),
        ];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_markdown(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();

        // Both type_hint groups must appear as ## headings.
        assert!(
            output.contains("## fact"),
            "output must contain ## fact header"
        );
        assert!(
            output.contains("## instruction"),
            "output must contain ## instruction header"
        );
    }

    #[test]
    fn test_markdown_output_blockquoted_content() {
        let memories = vec![make_memory("Some fact content here", "fact")];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_markdown(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        // Content must appear as blockquote.
        assert!(output.contains("> Some fact content here"));
    }

    #[test]
    fn test_markdown_groups_by_type_hint() {
        let memories = vec![
            make_memory("First fact memory", "fact"),
            make_memory("Prefer dark mode", "preference"),
            make_memory("Second fact memory", "fact"),
        ];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_markdown(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();

        // Both fact memories should be in the ## fact section.
        let fact_pos = output.find("## fact").unwrap();
        let pref_pos = output.find("## preference").unwrap();

        // fact section comes first alphabetically.
        assert!(fact_pos < pref_pos);

        // Both fact memories' content should appear.
        assert!(output.contains("First fact memory"));
        assert!(output.contains("Second fact memory"));
        assert!(output.contains("Prefer dark mode"));
    }

    #[test]
    fn test_markdown_empty_memories() {
        let opts = ExportOpts::default();
        let mut buf = Vec::new();
        write_markdown(&mut buf, &[], &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("# Memory Archive"));
        assert!(output.contains("*No memories exported.*"));
    }
}
