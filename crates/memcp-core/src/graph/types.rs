use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Discriminates the semantic class of an entity relationship.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RelationshipType {
    Temporal,
    Entity,
    Semantic,
    Causal,
}

impl RelationshipType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationshipType::Temporal => "temporal",
            RelationshipType::Entity => "entity",
            RelationshipType::Semantic => "semantic",
            RelationshipType::Causal => "causal",
        }
    }
}

impl std::fmt::Display for RelationshipType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A canonical entity node in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub id: Uuid,
    pub name: String,
    pub entity_type: String,
    /// JSON array of alternative names for this entity.
    pub aliases: serde_json::Value,
    /// Extensible metadata JSONB.
    pub metadata: serde_json::Value,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// A directed, typed relationship between two entity nodes.
///
/// Relationships are never deleted — only invalidated by setting `invalid_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRelationship {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub object_id: Uuid,
    /// Predicate label (e.g., "works_at", "located_in").
    pub predicate: String,
    pub relationship_type: String,
    pub weight: f64,
    pub valid_from: DateTime<Utc>,
    /// When set, this relationship is no longer active.
    pub invalid_at: Option<DateTime<Utc>>,
    /// ID of the relationship that superseded this one, if any.
    pub superseded_by: Option<Uuid>,
    /// Memory that is the source of this relationship.
    pub source_memory_id: Option<String>,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

/// An attribute/value fact about an entity, with temporal validity.
///
/// Facts are never deleted — only invalidated by setting `invalid_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityFact {
    pub id: Uuid,
    pub entity_id: Uuid,
    /// Attribute name (e.g., "job_title", "location").
    pub attribute: String,
    /// Attribute value as JSONB.
    pub value: serde_json::Value,
    pub valid_from: DateTime<Utc>,
    /// When set, this fact is no longer current.
    pub invalid_at: Option<DateTime<Utc>>,
    pub source_memory_id: Option<String>,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

/// Multiple active facts with the same attribute on a single entity.
///
/// Indicates conflicting scalar values that may require resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarContradiction {
    pub entity_id: Uuid,
    pub attribute: String,
    pub conflicting_facts: Vec<EntityFact>,
}

/// Two active relationships between the same entity pair whose predicates are
/// registered as contradictions in `predicate_contradictions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipContradiction {
    pub subject_id: Uuid,
    pub object_id: Uuid,
    pub relationship_a: EntityRelationship,
    pub relationship_b: EntityRelationship,
}

/// Full contradiction scan result for a single entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionReport {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub scalar: Vec<ScalarContradiction>,
    pub relationship: Vec<RelationshipContradiction>,
    pub has_contradictions: bool,
}

/// A link between an entity and the memory that mentions it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMention {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub memory_id: String,
    /// Surrounding text snippet from the source memory.
    pub context_snippet: Option<String>,
    pub created_at: DateTime<Utc>,
}
