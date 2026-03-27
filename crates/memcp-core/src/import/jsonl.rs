//! JSONL reader — imports memories from JSONL files (round-trip format).
//!
//! Each line is a JSON object with fields matching the Memory struct.
//! Lines with `"record_type": "entity"|"relationship"|"fact"` are graph records
//! and are handled by the graph import path. Lines without `record_type` (or with
//! `"record_type": "memory"`) are imported as memories — preserving full backward
//! compatibility with existing JSONL exports.
//!
//! Import order: memories → entities → facts → relationships (since
//! relationships reference entities by name).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::{DiscoveredSource, ImportChunk, ImportOpts, ImportSource, ImportSourceKind};

/// Deserializable representation of a single JSONL memory line.
#[derive(Debug, Deserialize, Serialize)]
struct JsonlMemoryLine {
    content: String,
    type_hint: Option<String>,
    source: Option<String>,
    tags: Option<Vec<String>>,
    created_at: Option<String>,
    actor: Option<String>,
    project: Option<String>,
    // Embedding fields for zero-cost reuse (round-trip with --include-embeddings).
    embedding: Option<Vec<f32>>,
    embedding_model: Option<String>,
}

/// Entity record produced by `memcp export --include-graph`.
#[derive(Debug, Deserialize)]
struct JsonlEntityLine {
    name: String,
    entity_type: String,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Relationship record produced by `memcp export --include-graph`.
#[derive(Debug, Deserialize)]
struct JsonlEntityRef {
    name: String,
    #[serde(rename = "type")]
    entity_type: String,
}

#[derive(Debug, Deserialize)]
struct JsonlRelationshipLine {
    subject: JsonlEntityRef,
    object: JsonlEntityRef,
    predicate: String,
    relationship_type: String,
    #[serde(default = "default_weight")]
    weight: f64,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

/// Fact record produced by `memcp export --include-graph`.
#[derive(Debug, Deserialize)]
struct JsonlFactLine {
    entity: JsonlEntityRef,
    attribute: String,
    value: serde_json::Value,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_weight() -> f64 {
    1.0
}
fn default_confidence() -> f64 {
    1.0
}


/// Look up an entity ID: check the in-memory cache built from this import's upserted
/// entities first, then fall back to a DB lookup for entities that existed before the
/// import. This eliminates N+1 `find_entity_by_name` calls during relationship/fact
/// import.
async fn resolve_entity_id(
    name: &str,
    entity_type: &str,
    cache: &HashMap<(String, String), uuid::Uuid>,
    store: &crate::storage::store::postgres::PostgresMemoryStore,
) -> Result<Option<uuid::Uuid>> {
    let key = (name.to_lowercase(), entity_type.to_string());
    if let Some(&id) = cache.get(&key) {
        return Ok(Some(id));
    }
    // Cache miss: entity existed before this import — fall back to DB.
    match store.find_entity_by_name(name).await {
        Ok(Some(node)) => Ok(Some(node.id)),
        Ok(None) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Entity lookup failed for '{}': {}", name, e)),
    }
}

/// JSONL source reader. Implements ImportSource for .jsonl files.
pub struct JsonlReader;

#[async_trait]
impl ImportSource for JsonlReader {
    fn source_name(&self) -> &str {
        "jsonl"
    }

    fn source_kind(&self) -> ImportSourceKind {
        ImportSourceKind::Jsonl
    }

    /// JSONL is pre-curated — no hardcoded noise patterns.
    fn noise_patterns(&self) -> Vec<&'static str> {
        vec![]
    }

    /// No auto-discovery for generic JSONL files.
    async fn discover(&self) -> Result<Vec<DiscoveredSource>> {
        Ok(vec![])
    }

    /// Read all memory chunks from the given JSONL file.
    ///
    /// Lines with `record_type` set to "entity", "relationship", or "fact" are
    /// silently skipped here — they are handled by `import_graph`. This maintains
    /// backward compatibility: old JSONL files without `record_type` parse exactly
    /// as before.
    async fn read_chunks(&self, path: &Path, opts: &ImportOpts) -> Result<Vec<ImportChunk>> {
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open JSONL file {:?}: {}", path, e))?;
        let reader = BufReader::new(file);

        let mut chunks = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!("Failed to read line {}: {}", line_num + 1, e);
                    continue;
                }
            };

            let trimmed = line.trim();
            // Skip blank lines and comment lines.
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            // Peek at record_type to skip graph records without a spurious warning.
            if let Ok(probe) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(rt) = probe.get("record_type").and_then(|v| v.as_str()) {
                    if matches!(rt, "entity" | "relationship" | "fact") {
                        continue;
                    }
                }
            }

            let parsed: JsonlMemoryLine = match serde_json::from_str(trimmed) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Line {}: JSON parse error: {}", line_num + 1, e);
                    continue;
                }
            };

            // Parse created_at from ISO 8601 string.
            let created_at = parsed.created_at.as_deref().and_then(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .ok()
            });

            // Apply opts.since filter.
            if let (Some(since), Some(ts)) = (opts.since, created_at) {
                if ts < since {
                    continue;
                }
            }

            let source = parsed
                .source
                .unwrap_or_else(|| "imported:jsonl".to_string());

            chunks.push(ImportChunk {
                content: parsed.content,
                type_hint: parsed.type_hint,
                source,
                tags: parsed.tags.unwrap_or_default(),
                created_at,
                actor: parsed.actor,
                embedding: parsed.embedding,
                embedding_model: parsed.embedding_model,
                project: parsed.project,
            });
        }

        Ok(chunks)
    }
}

/// Graph import result summary.
#[derive(Debug, Default)]
pub struct GraphImportResult {
    pub entities_upserted: usize,
    pub relationships_created: usize,
    pub facts_created: usize,
    pub skipped: usize,
}

impl JsonlReader {
    /// Import graph records (entities, relationships, facts) from a mixed JSONL file.
    ///
    /// Must be called after memory import so that any memory-linked entities are
    /// already resolvable. Processes in dependency order: entities first, then
    /// facts, then relationships (relationships reference two entities).
    pub async fn import_graph(
        &self,
        path: &Path,
        store: &crate::storage::store::postgres::PostgresMemoryStore,
    ) -> Result<GraphImportResult> {
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open JSONL file {:?}: {}", path, e))?;
        let reader = BufReader::new(file);

        let mut entities: Vec<JsonlEntityLine> = Vec::new();
        let mut relationships: Vec<JsonlRelationshipLine> = Vec::new();
        let mut facts: Vec<JsonlFactLine> = Vec::new();

        for (line_num, line_result) in reader.lines().enumerate() {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!("Failed to read line {}: {}", line_num + 1, e);
                    continue;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }

            let probe: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Line {}: JSON parse error: {}", line_num + 1, e);
                    continue;
                }
            };

            match probe.get("record_type").and_then(|v| v.as_str()) {
                Some("entity") => {
                    match serde_json::from_value::<JsonlEntityLine>(probe) {
                        Ok(r) => entities.push(r),
                        Err(e) => tracing::warn!("Line {}: entity parse error: {}", line_num + 1, e),
                    }
                }
                Some("relationship") => {
                    match serde_json::from_value::<JsonlRelationshipLine>(probe) {
                        Ok(r) => relationships.push(r),
                        Err(e) => tracing::warn!("Line {}: relationship parse error: {}", line_num + 1, e),
                    }
                }
                Some("fact") => {
                    match serde_json::from_value::<JsonlFactLine>(probe) {
                        Ok(r) => facts.push(r),
                        Err(e) => tracing::warn!("Line {}: fact parse error: {}", line_num + 1, e),
                    }
                }
                // Not a graph record — skip (already handled by read_chunks).
                _ => continue,
            }
        }

        let mut result = GraphImportResult::default();

        // 1. Upsert entities and build a (lowercase_name, entity_type) -> id cache to
        //    eliminate N+1 find_entity_by_name calls during fact/relationship import.
        let mut entity_cache: HashMap<(String, String), uuid::Uuid> =
            HashMap::with_capacity(entities.len());
        for entity in &entities {
            match store.upsert_entity(&entity.name, &entity.entity_type, &entity.aliases).await {
                Ok(node) => {
                    entity_cache.insert(
                        (entity.name.to_lowercase(), entity.entity_type.clone()),
                        node.id,
                    );
                    result.entities_upserted += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to upsert entity '{}': {}", entity.name, e);
                    result.skipped += 1;
                }
            }
        }

        // 2. Create facts (entity must exist).
        for fact in &facts {
            let entity_id = match resolve_entity_id(
                &fact.entity.name,
                &fact.entity.entity_type,
                &entity_cache,
                store,
            )
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    tracing::warn!(
                        "Fact references unknown entity '{}' — skipping",
                        fact.entity.name
                    );
                    result.skipped += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("{e}");
                    result.skipped += 1;
                    continue;
                }
            };
            match store
                .create_fact(entity_id, &fact.attribute, &fact.value, None, fact.confidence)
                .await
            {
                Ok(_) => result.facts_created += 1,
                Err(e) => {
                    tracing::warn!("Failed to create fact '{}': {}", fact.attribute, e);
                    result.skipped += 1;
                }
            }
        }

        // 3. Create relationships (both subject and object must exist).
        for rel in &relationships {
            let subject_id = match resolve_entity_id(
                &rel.subject.name,
                &rel.subject.entity_type,
                &entity_cache,
                store,
            )
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    tracing::warn!(
                        "Relationship subject '{}' not found — skipping",
                        rel.subject.name
                    );
                    result.skipped += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("{e}");
                    result.skipped += 1;
                    continue;
                }
            };
            let object_id = match resolve_entity_id(
                &rel.object.name,
                &rel.object.entity_type,
                &entity_cache,
                store,
            )
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => {
                    tracing::warn!(
                        "Relationship object '{}' not found — skipping",
                        rel.object.name
                    );
                    result.skipped += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("{e}");
                    result.skipped += 1;
                    continue;
                }
            };
            match store
                .create_relationship(
                    subject_id,
                    object_id,
                    &rel.predicate,
                    &rel.relationship_type,
                    rel.weight,
                    None,
                    rel.confidence,
                )
                .await
            {
                Ok(_) => result.relationships_created += 1,
                Err(e) => {
                    tracing::warn!(
                        "Failed to create relationship '{}': {}",
                        rel.predicate,
                        e
                    );
                    result.skipped += 1;
                }
            }
        }

        Ok(result)
    }
}

/// Discover JSONL files in a directory (helper for tests and --discover).
pub fn discover_jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_jsonl_file(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::with_suffix(".jsonl").unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[tokio::test]
    async fn test_read_valid_jsonl() {
        let file = make_jsonl_file(&[
            r#"{"content":"User prefers Rust for backend services due to memory safety"}"#,
            r#"{"content":"Dark mode is preferred for coding sessions","type_hint":"preference","tags":["ui","editor"]}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].type_hint, None);
        assert_eq!(chunks[1].type_hint, Some("preference".to_string()));
        assert_eq!(chunks[1].tags, vec!["ui".to_string(), "editor".to_string()]);
    }

    #[tokio::test]
    async fn test_skips_blank_lines() {
        let file = make_jsonl_file(&[
            r#"{"content":"First memory that should be long enough to pass noise filter"}"#,
            "",
            r#"{"content":"Second memory that should also pass the noise filter check"}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        assert_eq!(chunks.len(), 2);
    }

    #[tokio::test]
    async fn test_skips_invalid_json_lines() {
        let file = make_jsonl_file(&[
            r#"{"content":"Valid memory line with sufficient content to pass"}"#,
            r#"not valid json at all"#,
            r#"{"content":"Another valid memory line with enough content here"}"#,
        ]);

        let reader = JsonlReader;
        let opts = ImportOpts::default();
        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();

        // Invalid line is skipped, valid lines are returned.
        assert_eq!(chunks.len(), 2);
    }

    #[tokio::test]
    async fn test_since_filter() {
        let file = make_jsonl_file(&[
            r#"{"content":"Old memory from before the filter cutoff date","created_at":"2020-01-01T00:00:00Z"}"#,
            r#"{"content":"New memory from after the filter cutoff date here","created_at":"2025-01-01T00:00:00Z"}"#,
        ]);

        let reader = JsonlReader;
        let mut opts = ImportOpts::default();
        opts.since = Some(
            DateTime::parse_from_rfc3339("2023-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let chunks = reader.read_chunks(file.path(), &opts).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("New"));
    }

    #[tokio::test]
    async fn test_source_name() {
        let reader = JsonlReader;
        assert_eq!(reader.source_name(), "jsonl");
        assert_eq!(reader.source_kind(), ImportSourceKind::Jsonl);
    }
}
