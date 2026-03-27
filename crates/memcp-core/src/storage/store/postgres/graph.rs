//! Knowledge graph storage methods for PostgresMemoryStore.
//!
//! Covers entity upsert, relationship/fact creation and invalidation,
//! graph traversal via recursive CTE, and normalization status management.

use sqlx::Row;
use uuid::Uuid;

use super::PostgresMemoryStore;
use crate::errors::MemcpError;
use crate::graph::{
    ContradictionReport, EntityFact, EntityMention, EntityNode, EntityRelationship,
    RelationshipContradiction, ScalarContradiction,
};
use crate::store::Memory;

impl PostgresMemoryStore {
    // -------------------------------------------------------------------------
    // Entity operations
    // -------------------------------------------------------------------------

    /// Upsert an entity by (name, entity_type).
    ///
    /// On conflict, updates `last_seen_at` and merges any new aliases into the
    /// existing JSONB array. Returns the full entity row after upsert.
    pub async fn upsert_entity(
        &self,
        name: &str,
        entity_type: &str,
        aliases: &[String],
    ) -> Result<EntityNode, MemcpError> {
        let aliases_json = serde_json::json!(aliases);

        let row = sqlx::query(
            "INSERT INTO entities (name, entity_type, aliases) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (name, entity_type) DO UPDATE \
               SET last_seen_at = NOW(), \
                   aliases = ( \
                     SELECT jsonb_agg(DISTINCT elem) \
                     FROM jsonb_array_elements(entities.aliases || EXCLUDED.aliases) elem \
                   ) \
             RETURNING id, name, entity_type, aliases, metadata, first_seen_at, last_seen_at",
        )
        .bind(name)
        .bind(entity_type)
        .bind(&aliases_json)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to upsert entity: {}", e)))?;

        row_to_entity_node(&row)
    }

    /// Look up an entity by name (case-insensitive) or via aliases JSONB array.
    ///
    /// Returns the first match. Aliases are searched with a JSONB containment check
    /// after lowercasing the lookup term.
    pub async fn find_entity_by_name(&self, name: &str) -> Result<Option<EntityNode>, MemcpError> {
        let lower = name.to_lowercase();

        let row = sqlx::query(
            "SELECT id, name, entity_type, aliases, metadata, first_seen_at, last_seen_at \
             FROM entities \
             WHERE name ILIKE $1 \
                OR aliases @> to_jsonb($1::text) \
             LIMIT 1",
        )
        .bind(&lower)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to find entity: {}", e)))?;

        row.map(|r| row_to_entity_node(&r)).transpose()
    }

    // -------------------------------------------------------------------------
    // Relationship operations
    // -------------------------------------------------------------------------

    /// Create a new entity relationship.
    ///
    /// Does not check for existing active relationships — callers that want
    /// supersession semantics should invalidate the old relationship first via
    /// `invalidate_relationship()` and pass its ID as `superseded_by`.
    pub async fn create_relationship(
        &self,
        subject_id: Uuid,
        object_id: Uuid,
        predicate: &str,
        relationship_type: &str,
        weight: f64,
        source_memory_id: Option<&str>,
        confidence: f64,
    ) -> Result<EntityRelationship, MemcpError> {
        let row = sqlx::query(
            "INSERT INTO entity_relationships \
             (subject_id, object_id, predicate, relationship_type, weight, source_memory_id, confidence) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, subject_id, object_id, predicate, relationship_type, weight, \
                       valid_from, invalid_at, superseded_by, source_memory_id, confidence, created_at",
        )
        .bind(subject_id)
        .bind(object_id)
        .bind(predicate)
        .bind(relationship_type)
        .bind(weight)
        .bind(source_memory_id)
        .bind(confidence)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to create relationship: {}", e)))?;

        row_to_relationship(&row)
    }

    /// Invalidate a relationship by setting `invalid_at = NOW()`.
    ///
    /// Relationships are never deleted — this marks them as no longer current.
    /// If `superseded_by` is provided it is written alongside the invalidation timestamp.
    pub async fn invalidate_relationship(
        &self,
        id: Uuid,
        superseded_by: Option<Uuid>,
    ) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE entity_relationships \
             SET invalid_at = NOW(), superseded_by = COALESCE($2, superseded_by) \
             WHERE id = $1 AND invalid_at IS NULL",
        )
        .bind(id)
        .bind(superseded_by)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to invalidate relationship: {}", e)))?;

        Ok(())
    }

    /// Traverse the entity graph up to `max_depth` hops from `entity_id`.
    ///
    /// Uses a recursive CTE. Only active (non-invalidated) relationships are
    /// followed. Returns at most `limit` (entity, relationship) pairs.
    pub async fn get_entity_neighbors(
        &self,
        entity_id: Uuid,
        max_depth: i32,
        limit: i64,
    ) -> Result<Vec<(EntityNode, EntityRelationship)>, MemcpError> {
        let rows = sqlx::query(
            "WITH RECURSIVE graph(entity_id, rel_id, depth, path) AS ( \
               SELECT $1::uuid, NULL::uuid, 0, ARRAY[$1::uuid] \
               UNION ALL \
               SELECT \
                 CASE WHEN er.subject_id = g.entity_id THEN er.object_id ELSE er.subject_id END, \
                 er.id, \
                 g.depth + 1, \
                 g.path || CASE WHEN er.subject_id = g.entity_id THEN er.object_id ELSE er.subject_id END \
               FROM graph g \
               JOIN entity_relationships er \
                 ON (er.subject_id = g.entity_id OR er.object_id = g.entity_id) \
                 AND er.invalid_at IS NULL \
               WHERE g.depth < $2 \
                 AND NOT (CASE WHEN er.subject_id = g.entity_id THEN er.object_id ELSE er.subject_id END) = ANY(g.path) \
             ) \
             SELECT DISTINCT ON (e.id) \
               e.id, e.name, e.entity_type, e.aliases, e.metadata, e.first_seen_at, e.last_seen_at, \
               er.id AS rel_id, er.subject_id, er.object_id, er.predicate, er.relationship_type, \
               er.weight, er.valid_from, er.invalid_at, er.superseded_by, \
               er.source_memory_id, er.confidence, er.created_at AS rel_created_at \
             FROM graph g \
             JOIN entities e ON e.id = g.entity_id \
             JOIN entity_relationships er ON er.id = g.rel_id \
             WHERE g.rel_id IS NOT NULL \
               AND g.entity_id != $1 \
             LIMIT $3",
        )
        .bind(entity_id)
        .bind(max_depth)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get entity neighbors: {}", e)))?;

        rows.iter()
            .map(|row| {
                let node = row_to_entity_node(row)?;
                let rel = row_to_relationship_from_prefixed(row)?;
                Ok((node, rel))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    // -------------------------------------------------------------------------
    // Fact operations
    // -------------------------------------------------------------------------

    /// Create a new attribute/value fact for an entity.
    pub async fn create_fact(
        &self,
        entity_id: Uuid,
        attribute: &str,
        value: &serde_json::Value,
        source_memory_id: Option<&str>,
        confidence: f64,
    ) -> Result<EntityFact, MemcpError> {
        let row = sqlx::query(
            "INSERT INTO entity_facts (entity_id, attribute, value, source_memory_id, confidence) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, entity_id, attribute, value, valid_from, invalid_at, source_memory_id, confidence, created_at",
        )
        .bind(entity_id)
        .bind(attribute)
        .bind(value)
        .bind(source_memory_id)
        .bind(confidence)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to create fact: {}", e)))?;

        row_to_fact(&row)
    }

    /// Invalidate a fact by setting `invalid_at = NOW()`.
    ///
    /// Facts are never deleted — this marks them as no longer current.
    pub async fn invalidate_fact(&self, id: Uuid) -> Result<(), MemcpError> {
        sqlx::query(
            "UPDATE entity_facts SET invalid_at = NOW() WHERE id = $1 AND invalid_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to invalidate fact: {}", e)))?;

        Ok(())
    }

    /// Retrieve facts for an entity.
    ///
    /// When `current_only` is true, filters to facts where `invalid_at IS NULL`.
    pub async fn get_entity_facts(
        &self,
        entity_id: Uuid,
        current_only: bool,
    ) -> Result<Vec<EntityFact>, MemcpError> {
        let sql = if current_only {
            "SELECT id, entity_id, attribute, value, valid_from, invalid_at, source_memory_id, confidence, created_at \
             FROM entity_facts WHERE entity_id = $1 AND invalid_at IS NULL ORDER BY valid_from DESC"
        } else {
            "SELECT id, entity_id, attribute, value, valid_from, invalid_at, source_memory_id, confidence, created_at \
             FROM entity_facts WHERE entity_id = $1 ORDER BY valid_from DESC"
        };

        let rows = sqlx::query(sql)
            .bind(entity_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to get entity facts: {}", e)))?;

        rows.iter()
            .map(|row| row_to_fact(row))
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    // -------------------------------------------------------------------------
    // Mention operations
    // -------------------------------------------------------------------------

    /// Record that a memory mentions an entity.
    pub async fn create_mention(
        &self,
        entity_id: Uuid,
        memory_id: &str,
        context_snippet: Option<&str>,
    ) -> Result<EntityMention, MemcpError> {
        let row = sqlx::query(
            "INSERT INTO entity_mentions (entity_id, memory_id, context_snippet) \
             VALUES ($1, $2, $3) \
             RETURNING id, entity_id, memory_id, context_snippet, created_at",
        )
        .bind(entity_id)
        .bind(memory_id)
        .bind(context_snippet)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to create mention: {}", e)))?;

        row_to_mention(&row)
    }

    /// Retrieve all mentions of an entity with the source memory_id.
    ///
    /// Returns `(EntityMention, memory_id)` pairs. The memory_id is redundant
    /// with `EntityMention.memory_id` but included for ergonomic destructuring.
    pub async fn get_entity_mentions(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<(EntityMention, String)>, MemcpError> {
        let rows = sqlx::query(
            "SELECT id, entity_id, memory_id, context_snippet, created_at \
             FROM entity_mentions WHERE entity_id = $1 ORDER BY created_at DESC",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to get entity mentions: {}", e)))?;

        rows.iter()
            .map(|row| {
                let mention = row_to_mention(row)?;
                let memory_id = mention.memory_id.clone();
                Ok((mention, memory_id))
            })
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    // -------------------------------------------------------------------------
    // Normalization status management
    // -------------------------------------------------------------------------

    /// Update the `entity_normalization_status` column for a memory.
    ///
    /// Valid statuses: "pending", "complete", "failed".
    pub async fn update_normalization_status(
        &self,
        memory_id: &str,
        status: &str,
    ) -> Result<(), MemcpError> {
        sqlx::query("UPDATE memories SET entity_normalization_status = $2 WHERE id = $1")
            .bind(memory_id)
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                MemcpError::Storage(format!("Failed to update normalization status: {}", e))
            })?;

        Ok(())
    }

    /// Expand a query term through the entity graph and return associated memory IDs.
    ///
    /// Resolves `query` to an entity via case-insensitive name/alias lookup, then
    /// traverses up to 2 hops of active relationships to collect neighbor entities.
    /// Returns the memory IDs from `entity_mentions` for the seed entity and all
    /// neighbors, deduplicated.
    ///
    /// Returns an empty Vec when:
    /// - No entity matches the query (zero overhead path)
    /// - Entity tables are empty (backward compatible)
    pub async fn graph_expand_memory_ids(&self, query: &str) -> Result<Vec<String>, MemcpError> {
        let seed = match self.find_entity_by_name(query).await? {
            Some(entity) => entity,
            None => return Ok(vec![]),
        };

        let neighbors: Vec<(EntityNode, EntityRelationship)> =
            self.get_entity_neighbors(seed.id, 2, 20).await?;

        // Collect entity IDs to fetch mentions for: seed + all neighbors
        let mut entity_ids = Vec::with_capacity(neighbors.len() + 1);
        entity_ids.push(seed.id);
        for (neighbor, _rel) in &neighbors {
            entity_ids.push(neighbor.id);
        }

        let rows = sqlx::query(
            "SELECT DISTINCT memory_id FROM entity_mentions WHERE entity_id = ANY($1)",
        )
        .bind(&entity_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to expand memory ids: {}", e)))?;

        let memory_ids = rows
            .iter()
            .map(|row| {
                row.try_get::<String, _>("memory_id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))
            })
            .collect::<Result<Vec<_>, MemcpError>>()?;

        Ok(memory_ids)
    }

    // -------------------------------------------------------------------------
    // Contradiction detection
    // -------------------------------------------------------------------------

    /// Find attributes where an entity has more than one active (non-invalidated) fact.
    ///
    /// Returns one `ScalarContradiction` per conflicting attribute, with all
    /// active `EntityFact` rows for that attribute populated.
    pub async fn detect_scalar_contradictions(
        &self,
        entity_id: &Uuid,
    ) -> Result<Vec<ScalarContradiction>, MemcpError> {
        // Identify attributes that have more than one active fact.
        let conflict_rows = sqlx::query(
            "SELECT attribute \
             FROM entity_facts \
             WHERE entity_id = $1 AND invalid_at IS NULL \
             GROUP BY attribute \
             HAVING count(*) > 1",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to detect scalar contradictions: {}", e)))?;

        let mut contradictions = Vec::with_capacity(conflict_rows.len());

        for row in &conflict_rows {
            let attribute: String = row
                .try_get("attribute")
                .map_err(|e| MemcpError::Storage(e.to_string()))?;

            let fact_rows = sqlx::query(
                "SELECT id, entity_id, attribute, value, valid_from, invalid_at, \
                        source_memory_id, confidence, created_at \
                 FROM entity_facts \
                 WHERE entity_id = $1 AND attribute = $2 AND invalid_at IS NULL \
                 ORDER BY valid_from DESC",
            )
            .bind(entity_id)
            .bind(&attribute)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to fetch conflicting facts: {}", e)))?;

            let conflicting_facts = fact_rows
                .iter()
                .map(|r| row_to_fact(r))
                .collect::<Result<Vec<_>, MemcpError>>()?;

            contradictions.push(ScalarContradiction {
                entity_id: *entity_id,
                attribute,
                conflicting_facts,
            });
        }

        Ok(contradictions)
    }

    /// Find active relationship pairs between the same entity pair whose predicates
    /// are registered as contradictions in `predicate_contradictions`.
    ///
    /// The `entity_id` may appear as either subject or object of the relationships.
    pub async fn detect_relationship_contradictions(
        &self,
        entity_id: &Uuid,
    ) -> Result<Vec<RelationshipContradiction>, MemcpError> {
        let rows = sqlx::query(
            "SELECT \
               r1.id AS r1_id, r1.subject_id AS r1_subject, r1.object_id AS r1_object, \
               r1.predicate AS r1_predicate, r1.relationship_type AS r1_rel_type, \
               r1.weight AS r1_weight, r1.valid_from AS r1_valid_from, \
               r1.invalid_at AS r1_invalid_at, r1.superseded_by AS r1_superseded_by, \
               r1.source_memory_id AS r1_source, r1.confidence AS r1_confidence, \
               r1.created_at AS r1_created_at, \
               r2.id AS r2_id, r2.subject_id AS r2_subject, r2.object_id AS r2_object, \
               r2.predicate AS r2_predicate, r2.relationship_type AS r2_rel_type, \
               r2.weight AS r2_weight, r2.valid_from AS r2_valid_from, \
               r2.invalid_at AS r2_invalid_at, r2.superseded_by AS r2_superseded_by, \
               r2.source_memory_id AS r2_source, r2.confidence AS r2_confidence, \
               r2.created_at AS r2_created_at \
             FROM entity_relationships r1 \
             JOIN entity_relationships r2 \
               ON r1.subject_id = r2.subject_id AND r1.object_id = r2.object_id \
             JOIN predicate_contradictions pc \
               ON pc.predicate_a = r1.predicate AND pc.predicate_b = r2.predicate \
             WHERE (r1.subject_id = $1 OR r1.object_id = $1) \
               AND r1.invalid_at IS NULL AND r2.invalid_at IS NULL \
               AND r1.id < r2.id",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to detect relationship contradictions: {}", e))
        })?;

        rows.iter()
            .map(|row| {
                let relationship_a = EntityRelationship {
                    id: row
                        .try_get("r1_id")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    subject_id: row
                        .try_get("r1_subject")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    object_id: row
                        .try_get("r1_object")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    predicate: row
                        .try_get("r1_predicate")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    relationship_type: row
                        .try_get("r1_rel_type")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    weight: row
                        .try_get("r1_weight")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    valid_from: row
                        .try_get("r1_valid_from")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    invalid_at: row
                        .try_get("r1_invalid_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    superseded_by: row
                        .try_get("r1_superseded_by")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    source_memory_id: row
                        .try_get("r1_source")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    confidence: row
                        .try_get("r1_confidence")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    created_at: row
                        .try_get("r1_created_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                };
                let relationship_b = EntityRelationship {
                    id: row
                        .try_get("r2_id")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    subject_id: row
                        .try_get("r2_subject")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    object_id: row
                        .try_get("r2_object")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    predicate: row
                        .try_get("r2_predicate")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    relationship_type: row
                        .try_get("r2_rel_type")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    weight: row
                        .try_get("r2_weight")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    valid_from: row
                        .try_get("r2_valid_from")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    invalid_at: row
                        .try_get("r2_invalid_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    superseded_by: row
                        .try_get("r2_superseded_by")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    source_memory_id: row
                        .try_get("r2_source")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    confidence: row
                        .try_get("r2_confidence")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                    created_at: row
                        .try_get("r2_created_at")
                        .map_err(|e| MemcpError::Storage(e.to_string()))?,
                };
                Ok(RelationshipContradiction {
                    subject_id: relationship_a.subject_id,
                    object_id: relationship_a.object_id,
                    relationship_a,
                    relationship_b,
                })
            })
            .collect::<Result<Vec<_>, MemcpError>>()
    }

    /// Run both scalar and relationship contradiction scans for an entity, returning
    /// a combined `ContradictionReport`.
    pub async fn detect_all_contradictions(
        &self,
        entity_id: &Uuid,
        entity_name: &str,
    ) -> Result<ContradictionReport, MemcpError> {
        let scalar = self.detect_scalar_contradictions(entity_id).await?;
        let relationship = self.detect_relationship_contradictions(entity_id).await?;
        let has_contradictions = !scalar.is_empty() || !relationship.is_empty();
        Ok(ContradictionReport {
            entity_id: *entity_id,
            entity_name: entity_name.to_owned(),
            scalar,
            relationship,
            has_contradictions,
        })
    }

    /// Fetch memories that are ready for entity normalization.
    ///
    /// Returns memories where extraction is complete but graph normalization is
    /// still pending, ordered by creation time for FIFO processing.
    pub async fn get_pending_normalization(&self, limit: i64) -> Result<Vec<Memory>, MemcpError> {
        let rows = sqlx::query(
            "SELECT * FROM memories \
             WHERE entity_normalization_status = 'pending' \
               AND extraction_status = 'complete' \
               AND deleted_at IS NULL \
             ORDER BY created_at ASC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            MemcpError::Storage(format!("Failed to fetch pending normalization: {}", e))
        })?;

        rows.iter()
            .map(|row| super::row_to_memory(row))
            .collect::<Result<Vec<_>, MemcpError>>()
    }
}

// -------------------------------------------------------------------------
// Row mapping helpers
// -------------------------------------------------------------------------

fn row_to_entity_node(row: &sqlx::postgres::PgRow) -> Result<EntityNode, MemcpError> {
    Ok(EntityNode {
        id: row
            .try_get("id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        name: row
            .try_get("name")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        entity_type: row
            .try_get("entity_type")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        aliases: row
            .try_get("aliases")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        metadata: row
            .try_get("metadata")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        first_seen_at: row
            .try_get("first_seen_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        last_seen_at: row
            .try_get("last_seen_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
    })
}

fn row_to_relationship(row: &sqlx::postgres::PgRow) -> Result<EntityRelationship, MemcpError> {
    Ok(EntityRelationship {
        id: row
            .try_get("id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        subject_id: row
            .try_get("subject_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        object_id: row
            .try_get("object_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        predicate: row
            .try_get("predicate")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        relationship_type: row
            .try_get("relationship_type")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        weight: row
            .try_get("weight")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        valid_from: row
            .try_get("valid_from")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        invalid_at: row
            .try_get("invalid_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        superseded_by: row
            .try_get("superseded_by")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        source_memory_id: row
            .try_get("source_memory_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        confidence: row
            .try_get("confidence")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
    })
}

/// Map a row from the neighbors CTE query where relationship columns are prefixed.
///
/// The CTE query aliases `er.created_at` as `rel_created_at` and `er.id` as `rel_id`
/// to avoid column name collisions with the entity columns from the same row.
fn row_to_relationship_from_prefixed(
    row: &sqlx::postgres::PgRow,
) -> Result<EntityRelationship, MemcpError> {
    Ok(EntityRelationship {
        id: row
            .try_get("rel_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        subject_id: row
            .try_get("subject_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        object_id: row
            .try_get("object_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        predicate: row
            .try_get("predicate")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        relationship_type: row
            .try_get("relationship_type")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        weight: row
            .try_get("weight")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        valid_from: row
            .try_get("valid_from")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        invalid_at: row
            .try_get("invalid_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        superseded_by: row
            .try_get("superseded_by")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        source_memory_id: row
            .try_get("source_memory_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        confidence: row
            .try_get("confidence")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row
            .try_get("rel_created_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
    })
}

fn row_to_fact(row: &sqlx::postgres::PgRow) -> Result<EntityFact, MemcpError> {
    Ok(EntityFact {
        id: row
            .try_get("id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        entity_id: row
            .try_get("entity_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        attribute: row
            .try_get("attribute")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        value: row
            .try_get("value")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        valid_from: row
            .try_get("valid_from")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        invalid_at: row
            .try_get("invalid_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        source_memory_id: row
            .try_get("source_memory_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        confidence: row
            .try_get("confidence")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
    })
}

fn row_to_mention(row: &sqlx::postgres::PgRow) -> Result<EntityMention, MemcpError> {
    Ok(EntityMention {
        id: row
            .try_get("id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        entity_id: row
            .try_get("entity_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        memory_id: row
            .try_get("memory_id")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        context_snippet: row
            .try_get("context_snippet")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| MemcpError::Storage(e.to_string()))?,
    })
}
