//! JSONL formatter for the export pipeline.
//!
//! Produces one JSON object per line. All fields from Memory are included.
//! Optional --include-state adds FSRS/salience fields.
//! Optional --include-embeddings adds embedding vector as a JSON array.
//!
//! This format is the canonical round-trip format: `memcp export --format jsonl`
//! output can be re-imported via `memcp import jsonl`.

use std::io::Write;

use anyhow::Result;
use serde_json::json;

use super::{ExportOpts, ExportableMemory};

/// Write memories as JSONL — one JSON object per line.
///
/// Each line is a valid JSON object. Lines are separated by `\n`.
/// The output is compatible with `memcp import jsonl` for full round-trip fidelity.
pub fn write_jsonl(
    writer: &mut dyn Write,
    memories: &[ExportableMemory],
    opts: &ExportOpts,
) -> Result<()> {
    for mem in memories {
        let tags: Vec<String> = mem
            .tags
            .as_ref()
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        let mut obj = json!({
            "id": mem.id,
            "content": mem.content,
            "type_hint": mem.type_hint,
            "source": mem.source,
            "tags": tags,
            "created_at": mem.created_at.to_rfc3339(),
            "actor": mem.actor,
            "actor_type": mem.actor_type,
            "audience": mem.audience,
            "project": mem.project,
            "event_time": mem.event_time.map(|t| t.to_rfc3339()),
            "event_time_precision": mem.event_time_precision,
        });

        if opts.include_state {
            let map = obj.as_object_mut().unwrap();
            map.insert("stability".to_string(), json!(mem.stability));
            map.insert("difficulty".to_string(), json!(mem.difficulty));
            map.insert("reinforcement_count".to_string(), json!(mem.reinforcement_count));
            if let Some(ts) = mem.last_reinforced_at {
                map.insert("last_reinforced_at".to_string(), json!(ts.to_rfc3339()));
            }
        }

        if opts.include_embeddings {
            if let Some(ref embedding) = mem.embedding {
                let map = obj.as_object_mut().unwrap();
                map.insert("embedding".to_string(), json!(embedding));
                map.insert("embedding_model".to_string(), json!(mem.embedding_model));
            }
        }

        let line = serde_json::to_string(&obj)?;
        writeln!(writer, "{}", line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_memory(content: &str, type_hint: &str, tags: Vec<&str>) -> ExportableMemory {
        ExportableMemory {
            id: "test-id-1".to_string(),
            content: content.to_string(),
            type_hint: type_hint.to_string(),
            source: "test".to_string(),
            tags: Some(serde_json::Value::Array(
                tags.into_iter().map(|t| serde_json::Value::String(t.to_string())).collect(),
            )),
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
    fn test_jsonl_output_basic() {
        let memories = vec![
            make_memory("Rust is great for memory safety", "fact", vec!["rust", "safety"]),
            make_memory("Dark mode preferred for coding", "preference", vec!["ui"]),
        ];
        let opts = ExportOpts::default();

        let mut buf = Vec::new();
        write_jsonl(&mut buf, &memories, &opts).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = output.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 2);

        // Each line must be valid JSON.
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("line must be valid JSON");
            assert!(parsed.get("content").is_some(), "content field missing");
            assert!(parsed.get("type_hint").is_some(), "type_hint field missing");
            assert!(parsed.get("tags").is_some(), "tags field missing");
            assert!(parsed.get("created_at").is_some(), "created_at field missing");
        }

        // Check specific values.
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["content"].as_str().unwrap(), "Rust is great for memory safety");
        assert_eq!(first["type_hint"].as_str().unwrap(), "fact");
        let tags = first["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].as_str().unwrap(), "rust");
    }

    #[test]
    fn test_jsonl_include_state() {
        let memories = vec![make_memory("fact content here", "fact", vec![])];
        let opts = ExportOpts {
            include_state: true,
            ..ExportOpts::default()
        };

        let mut buf = Vec::new();
        write_jsonl(&mut buf, &memories, &opts).unwrap();

        let line = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert!(parsed.get("stability").is_some(), "stability field missing when include_state=true");
        assert!(parsed.get("difficulty").is_some(), "difficulty field missing when include_state=true");
        assert!(parsed.get("reinforcement_count").is_some(), "reinforcement_count field missing");
    }

    #[test]
    fn test_jsonl_include_embeddings() {
        let mut mem = make_memory("content with embedding", "fact", vec![]);
        mem.embedding = Some(vec![0.1_f32, 0.2, 0.3]);
        mem.embedding_model = Some("all-minilm-l6-v2".to_string());

        let opts = ExportOpts {
            include_embeddings: true,
            ..ExportOpts::default()
        };

        let mut buf = Vec::new();
        write_jsonl(&mut buf, &[mem], &opts).unwrap();

        let line = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert!(parsed.get("embedding").is_some(), "embedding field missing when include_embeddings=true");
        let embedding = parsed["embedding"].as_array().unwrap();
        assert_eq!(embedding.len(), 3);
    }

    #[test]
    fn test_jsonl_empty_memories() {
        let opts = ExportOpts::default();
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &[], &opts).unwrap();
        assert_eq!(buf.len(), 0);
    }
}
