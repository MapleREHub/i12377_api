//! HTTP routes.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use tracing::info;

use crate::config::Config;
use crate::error::ApiError;
use crate::models::{HealthResponse, QueryRequest, QueryResponse};
use crate::orchestrator;

/// `GET /health` — liveness probe.
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(HealthResponse::default()))
}

/// `POST /query` — query a single report by retrieval code.
pub async fn query(
    State(cfg): State<Config>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, ApiError> {
    let code = req.retrieval_code.trim();
    if code.is_empty() {
        return Err(ApiError::EmptyCode);
    }
    if code.len() > 64 {
        return Err(ApiError::CodeTooLong);
    }
    info!(retrieval_code = %code, "received query request");
    let resp = orchestrator::do_query(&cfg, code).await?;
    Ok(Json(resp))
}