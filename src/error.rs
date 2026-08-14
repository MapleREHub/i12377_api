//! Error types for the API layer.
//!
//! `ApiError` maps cleanly onto HTTP responses via `IntoResponse`. Errors that
//! the orchestrator recovers from (e.g. captcha mis-solve) are *not* surfaced
//! as `ApiError` — they become `QueryResponse { success: false, .. }`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Top-level error type for the API surface.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("retrieval code cannot be empty")]
    EmptyCode,
    #[error("retrieval code too long (max 64 chars)")]
    CodeTooLong,
    #[error("upstream network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("upstream returned non-JSON response")]
    BadJson,
    #[error("captcha recognition failed after {0} attempts")]
    CaptchaExhausted(u32),
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::EmptyCode | ApiError::CodeTooLong => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            ApiError::Network(_) => (
                StatusCode::BAD_GATEWAY,
                "upstream unreachable".to_string(),
            ),
            ApiError::BadJson => (
                StatusCode::BAD_GATEWAY,
                "bad upstream response".to_string(),
            ),
            ApiError::CaptchaExhausted(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "captcha solve failed; please retry".to_string(),
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_string(),
            ),
        };
        (status, Json(json!({"success": false, "error": msg}))).into_response()
    }
}