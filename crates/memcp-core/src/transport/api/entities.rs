//! Entity graph HTTP endpoints for the knowledge graph.
//!
//! Routes:
//!   GET /v1/entities                   — list entities with optional type filter
//!   GET /v1/entities/:id               — single entity with current facts + mention count
//!   GET /v1/entities/:id/relationships — neighbors of an entity with traversal depth

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use super::types::error_json;
use crate::errors::MemcpError;
use crate::graph::EntityNode;
use crate::store::postgres::PostgresMemoryStore;
use crate::transport::health::AppState;

#[derive(Debug, Deserialize)]
pub struct ListEntitiesQuery {
    /// Filter by entity_type (e.g. "person", "org"). Absent = all types.
    pub entity_type: Option<String>,
    /// Max entities to return (default 100, capped at 500).
    pub limit: Option<i64>,
    /// Offset for pagination (default 0).
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RelationshipsQuery {
    /// Traversal depth 1-3 (default 1).
    pub depth: Option<i32>,
    /// Max results (default 50, capped at 200).
    pub limit: Option<i64>,
}

/// GET /v1/entities — list entities with optional type filter and pagination.
pub async fn list_entities_handler(
    State(state): State<AppState>,
    Query(params): Query<ListEntitiesQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(error_json("store not available")),
            )
        }
    };

    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);

    let result: Result<Vec<crate::graph::EntityNode>, crate::errors::MemcpError> =
        list_entities(&store, params.entity_type.as_deref(), limit, offset).await;

    match result {
        Ok(entities) => {
            let count = entities.len();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "entities": entities,
                    "count": count,
                    "limit": limit,
                    "offset": offset,
                })),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "list_entities query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Failed to list entities: {}", e))),
            )
        }
    }
}

/// GET /v1/entities/:id — entity detail with current facts and mention count.
pub async fn get_entity_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(error_json("store not available")),
            )
        }
    };

    let entity = match fetch_entity_by_id(&store, id).await {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_json("entity not found"))),
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "get_entity query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Failed to fetch entity: {}", e))),
            );
        }
    };

    let facts = match store.get_entity_facts(id, true).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "get_entity_facts failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Failed to fetch entity facts: {}", e))),
            );
        }
    };

    let mention_count = match count_entity_mentions(&store, id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "count_entity_mentions failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!(
                    "Failed to count entity mentions: {}",
                    e
                ))),
            );
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "entity": entity,
            "facts": facts,
            "mention_count": mention_count,
        })),
    )
}

/// GET /v1/entities/:id/relationships — entity neighbors with optional traversal depth.
pub async fn get_entity_relationships_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<RelationshipsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(error_json("store not available")),
            )
        }
    };

    let depth = params.depth.unwrap_or(1).clamp(1, 3);
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let neighbors = match store.get_entity_neighbors(id, depth, limit).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "get_entity_neighbors failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!(
                    "Failed to fetch entity relationships: {}",
                    e
                ))),
            );
        }
    };

    let items: Vec<serde_json::Value> = neighbors
        .into_iter()
        .map(|(entity, relationship)| {
            serde_json::json!({
                "entity": entity,
                "relationship": relationship,
            })
        })
        .collect();

    let count = items.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "relationships": items,
            "count": count,
            "depth": depth,
        })),
    )
}

/// GET /v1/entities/:id/contradictions — full contradiction scan for an entity.
pub async fn get_entity_contradictions_handler(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(error_json("store not available")),
            )
        }
    };

    // Verify entity exists and get its name for the report.
    let entity = match fetch_entity_by_id(&store, id).await {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(error_json("entity not found"))),
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "get_entity_contradictions: entity lookup failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Failed to fetch entity: {}", e))),
            );
        }
    };

    match store.detect_all_contradictions(&id, &entity.name).await {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(v) => (StatusCode::OK, Json(v)),
            Err(e) => {
                tracing::warn!(error = %e, entity_id = %id, "Failed to serialize contradiction report");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json("Failed to serialize contradiction report")),
                )
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, entity_id = %id, "detect_all_contradictions failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!(
                    "Failed to detect contradictions: {}",
                    e
                ))),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Internal query helpers
// ---------------------------------------------------------------------------

async fn list_entities(
    store: &Arc<PostgresMemoryStore>,
    entity_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<EntityNode>, MemcpError> {
    let rows = if let Some(et) = entity_type {
        sqlx::query(
            "SELECT id, name, entity_type, aliases, metadata, first_seen_at, last_seen_at \
             FROM entities WHERE entity_type = $1 ORDER BY last_seen_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(et)
        .bind(limit)
        .bind(offset)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to list entities by type: {}", e)))?
    } else {
        sqlx::query(
            "SELECT id, name, entity_type, aliases, metadata, first_seen_at, last_seen_at \
             FROM entities ORDER BY last_seen_at DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to list entities: {}", e)))?
    };

    rows.iter().map(|r| map_entity_node_row(r)).collect()
}

async fn fetch_entity_by_id(
    store: &Arc<PostgresMemoryStore>,
    id: Uuid,
) -> Result<Option<EntityNode>, MemcpError> {
    let row = sqlx::query(
        "SELECT id, name, entity_type, aliases, metadata, first_seen_at, last_seen_at \
         FROM entities WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await
    .map_err(|e| MemcpError::Storage(format!("Failed to fetch entity by id: {}", e)))?;

    row.map(|r| map_entity_node_row(&r)).transpose()
}

async fn count_entity_mentions(
    store: &Arc<PostgresMemoryStore>,
    entity_id: Uuid,
) -> Result<i64, MemcpError> {
    let row =
        sqlx::query("SELECT COUNT(*) AS mention_count FROM entity_mentions WHERE entity_id = $1")
            .bind(entity_id)
            .fetch_one(store.pool())
            .await
            .map_err(|e| MemcpError::Storage(format!("Failed to count mentions: {}", e)))?;

    row.try_get("mention_count")
        .map_err(|e| MemcpError::Storage(e.to_string()))
}

fn map_entity_node_row(row: &sqlx::postgres::PgRow) -> Result<EntityNode, MemcpError> {
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
