//! GET /v1/export — export memories in the requested format.
//!
//! Query parameters mirror `ExportOpts`:
//!   format           — "jsonl" (default), "csv", "markdown"
//!   project          — filter by project scope
//!   tag              — can be repeated; memories must have ALL specified tags
//!   since            — ISO 8601 timestamp filter
//!   include_embeddings — "true" to include embedding vectors
//!   include_state    — "true" to include FSRS state
//!
//! Returns raw export data with appropriate Content-Type.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::import::export::{ExportEngine, ExportFormat, ExportOpts};
use crate::transport::health::AppState;

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "tag")]
    pub tags: Option<Vec<String>>,
    pub since: Option<String>,
    pub include_embeddings: Option<String>,
    pub include_state: Option<String>,
}

/// GET /v1/export — export memories to the requested format.
pub async fn export_handler(
    State(state): State<AppState>,
    Query(params): Query<ExportQuery>,
) -> impl IntoResponse {
    let store = match &state.store {
        Some(s) => s.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                r#"{"error":"store not available"}"#.to_string(),
            )
                .into_response();
        }
    };

    let format_str = params.format.as_deref().unwrap_or("jsonl");
    let format = match ExportFormat::from_str(format_str) {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                format!("{{\"error\":\"{}\"}}", e),
            )
                .into_response();
        }
    };

    let since_dt: Option<DateTime<Utc>> = if let Some(ref s) = params.since {
        match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CONTENT_TYPE, "application/json")],
                    format!("{{\"error\":\"invalid since timestamp: {}\"}}", e),
                )
                    .into_response();
            }
        }
    } else {
        None
    };

    let include_embeddings = params.include_embeddings.as_deref() == Some("true");
    let include_state = params.include_state.as_deref() == Some("true");

    let content_type = match format {
        ExportFormat::Jsonl => "application/x-ndjson",
        ExportFormat::Csv => "text/csv",
        ExportFormat::Markdown => "text/markdown",
    };

    let opts = ExportOpts {
        format,
        output: None,
        project: params.project,
        tags: params.tags,
        since: since_dt,
        include_embeddings,
        include_state,
    };

    let engine = ExportEngine::new(store);
    let mut buf: Vec<u8> = Vec::new();

    match engine.run_to_writer(&mut buf, &opts).await {
        Ok(_) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            String::from_utf8_lossy(&buf).into_owned(),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            format!("{{\"error\":\"export failed: {}\"}}", e),
        )
            .into_response(),
    }
}
