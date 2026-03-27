//! GET /v1/pipeline/health — aggregate counts for extraction + normalization pipeline.
//!
//! Returns counts grouped by extraction_status and entity_normalization_status,
//! plus a structured vs flat fact ratio for completed memories.

use axum::{extract::State, http::StatusCode, Json};
use sqlx::Row;

use super::types::error_json;
use crate::transport::health::AppState;

/// GET /v1/pipeline/health — pipeline status aggregates.
///
/// Returns:
/// ```json
/// {
///   "extraction": {"pending": N, "complete": N, "failed": N, "skipped": N},
///   "normalization": {"pending": N, "complete": N, "failed": N},
///   "facts": {"structured": N, "flat": N, "none": N},
///   "total_memories": N
/// }
/// ```
pub async fn pipeline_health_handler(
    State(state): State<AppState>,
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

    let pool = store.pool();

    // Aggregate by extraction_status × entity_normalization_status in one pass.
    let status_rows = match sqlx::query(
        "SELECT \
           extraction_status, \
           entity_normalization_status, \
           count(*) AS count \
         FROM memories \
         GROUP BY extraction_status, entity_normalization_status",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "pipeline health status query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Status query failed: {}", e))),
            );
        }
    };

    // Fact format counts for memories where extraction completed.
    let fact_row = match sqlx::query(
        "SELECT \
           count(*) FILTER (WHERE extracted_facts::text LIKE '%\"entity\"%') AS structured_count, \
           count(*) FILTER (WHERE extracted_facts IS NOT NULL AND extracted_facts::text NOT LIKE '%\"entity\"%') AS flat_count, \
           count(*) FILTER (WHERE extracted_facts IS NULL) AS no_facts_count \
         FROM memories \
         WHERE extraction_status = 'complete'",
    )
    .fetch_one(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "pipeline health facts query failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_json(&format!("Facts query failed: {}", e))),
            );
        }
    };

    // Accumulate extraction and normalization counts from the grouped rows.
    let mut extraction_pending: i64 = 0;
    let mut extraction_complete: i64 = 0;
    let mut extraction_failed: i64 = 0;
    let mut extraction_skipped: i64 = 0;

    let mut norm_pending: i64 = 0;
    let mut norm_complete: i64 = 0;
    let mut norm_failed: i64 = 0;

    let mut total: i64 = 0;

    for row in &status_rows {
        let ext_status: String = row.try_get("extraction_status").unwrap_or_default();
        let norm_status: String = row
            .try_get("entity_normalization_status")
            .unwrap_or_default();
        let count: i64 = row.try_get("count").unwrap_or(0);

        total += count;

        match ext_status.as_str() {
            "pending" => extraction_pending += count,
            "complete" => extraction_complete += count,
            "failed" => extraction_failed += count,
            "skipped" => extraction_skipped += count,
            _ => {}
        }

        match norm_status.as_str() {
            "pending" => norm_pending += count,
            "complete" => norm_complete += count,
            "failed" => norm_failed += count,
            _ => {}
        }
    }

    let structured: i64 = fact_row.try_get("structured_count").unwrap_or(0);
    let flat: i64 = fact_row.try_get("flat_count").unwrap_or(0);
    let no_facts: i64 = fact_row.try_get("no_facts_count").unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "extraction": {
                "pending": extraction_pending,
                "complete": extraction_complete,
                "failed": extraction_failed,
                "skipped": extraction_skipped
            },
            "normalization": {
                "pending": norm_pending,
                "complete": norm_complete,
                "failed": norm_failed
            },
            "facts": {
                "structured": structured,
                "flat": flat,
                "none": no_facts
            },
            "total_memories": total
        })),
    )
}
