//! GET /v1/graph — full subgraph for knowledge graph visualization.
//!
//! Returns nodes + edges in the format expected by v-network-graph (Vue frontend).
//! Nodes are entities; edges are active (non-invalidated) relationships.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use super::types::error_json;
use crate::errors::MemcpError;
use crate::store::postgres::PostgresMemoryStore;
use crate::transport::health::AppState;

#[derive(Debug, Deserialize)]
pub struct GraphQuery {
    /// Filter nodes by entity_type. Absent = all types.
    pub entity_type: Option<String>,
    /// Filter edges by relationship_type. Absent = all types.
    pub relationship_type: Option<String>,
    /// Max nodes to return (default 200, capped at 1000).
    pub limit: Option<i64>,
}

/// A node in the visualization graph — one per entity.
#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: Uuid,
    pub name: String,
    pub entity_type: String,
    pub fact_count: i64,
    pub mention_count: i64,
}

/// A directed edge in the visualization graph — one per active relationship.
#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub id: Uuid,
    /// subject entity id (edge source)
    pub source: Uuid,
    /// object entity id (edge target)
    pub target: Uuid,
    pub predicate: String,
    pub relationship_type: String,
    pub weight: f64,
    pub confidence: f64,
}

/// GET /v1/graph — subgraph suitable for v-network-graph visualization.
///
/// Returns `{ nodes, edges }` where nodes are entities and edges are active
/// (non-invalidated) relationships. Optionally filtered by entity_type and/or
/// relationship_type.
pub async fn graph_handler(
    State(state): State<AppState>,
    Query(params): Query<GraphQuery>,
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

    let node_limit = params.limit.unwrap_or(200).clamp(1, 1000);

    let nodes = match fetch_graph_nodes(&store, params.entity_type.as_deref(), node_limit).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "graph nodes query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Failed to fetch graph nodes: {}", e))),
            );
        }
    };

    // Collect node IDs so we only return edges between visible nodes.
    let node_ids: Vec<Uuid> = nodes.iter().map(|n| n.id).collect();

    let edges =
        match fetch_graph_edges(&store, &node_ids, params.relationship_type.as_deref()).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "graph edges query failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_json(&format!("Failed to fetch graph edges: {}", e))),
                );
            }
        };

    let node_count = nodes.len();
    let edge_count = edges.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "nodes": nodes,
            "edges": edges,
            "node_count": node_count,
            "edge_count": edge_count,
        })),
    )
}

// ---------------------------------------------------------------------------
// Internal query helpers
// ---------------------------------------------------------------------------

async fn fetch_graph_nodes(
    store: &Arc<PostgresMemoryStore>,
    entity_type: Option<&str>,
    limit: i64,
) -> Result<Vec<GraphNode>, MemcpError> {
    // Aggregate fact_count and mention_count per entity in one query.
    // LEFT JOINs ensure entities with zero facts or mentions are included.
    let rows = if let Some(et) = entity_type {
        sqlx::query(
            "SELECT e.id, e.name, e.entity_type, \
               COUNT(DISTINCT ef.id) FILTER (WHERE ef.invalid_at IS NULL) AS fact_count, \
               COUNT(DISTINCT em.id) AS mention_count \
             FROM entities e \
             LEFT JOIN entity_facts ef ON ef.entity_id = e.id \
             LEFT JOIN entity_mentions em ON em.entity_id = e.id \
             WHERE e.entity_type = $1 \
             GROUP BY e.id, e.name, e.entity_type \
             ORDER BY e.last_seen_at DESC \
             LIMIT $2",
        )
        .bind(et)
        .bind(limit)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch graph nodes by type: {}", e)))?
    } else {
        sqlx::query(
            "SELECT e.id, e.name, e.entity_type, \
               COUNT(DISTINCT ef.id) FILTER (WHERE ef.invalid_at IS NULL) AS fact_count, \
               COUNT(DISTINCT em.id) AS mention_count \
             FROM entities e \
             LEFT JOIN entity_facts ef ON ef.entity_id = e.id \
             LEFT JOIN entity_mentions em ON em.entity_id = e.id \
             GROUP BY e.id, e.name, e.entity_type \
             ORDER BY e.last_seen_at DESC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch graph nodes: {}", e)))?
    };

    rows.iter()
        .map(|row| {
            Ok(GraphNode {
                id: row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                name: row
                    .try_get("name")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                entity_type: row
                    .try_get("entity_type")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                fact_count: row
                    .try_get("fact_count")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                mention_count: row
                    .try_get("mention_count")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
            })
        })
        .collect()
}

async fn fetch_graph_edges(
    store: &Arc<PostgresMemoryStore>,
    node_ids: &[Uuid],
    relationship_type: Option<&str>,
) -> Result<Vec<GraphEdge>, MemcpError> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Only return edges where both endpoints are in the visible node set.
    // Uses ANY($1) for the UUID array bind.
    let rows = if let Some(rt) = relationship_type {
        sqlx::query(
            "SELECT id, subject_id, object_id, predicate, relationship_type, weight, confidence \
             FROM entity_relationships \
             WHERE invalid_at IS NULL \
               AND subject_id = ANY($1) \
               AND object_id = ANY($1) \
               AND relationship_type = $2",
        )
        .bind(node_ids)
        .bind(rt)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch graph edges by type: {}", e)))?
    } else {
        sqlx::query(
            "SELECT id, subject_id, object_id, predicate, relationship_type, weight, confidence \
             FROM entity_relationships \
             WHERE invalid_at IS NULL \
               AND subject_id = ANY($1) \
               AND object_id = ANY($1)",
        )
        .bind(node_ids)
        .fetch_all(store.pool())
        .await
        .map_err(|e| MemcpError::Storage(format!("Failed to fetch graph edges: {}", e)))?
    };

    rows.iter()
        .map(|row| {
            Ok(GraphEdge {
                id: row
                    .try_get("id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                source: row
                    .try_get("subject_id")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
                target: row
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
                confidence: row
                    .try_get("confidence")
                    .map_err(|e| MemcpError::Storage(e.to_string()))?,
            })
        })
        .collect()
}
