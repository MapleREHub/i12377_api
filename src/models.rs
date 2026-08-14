//! Request and response DTOs.
//!
//! Field names mirror the Python reference implementation so the API is
//! wire-compatible with existing clients.

use serde::{Deserialize, Serialize};

/// `POST /query` request body.
#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    /// 举报查询码（1–64 字符），如 `H026061219520245669A`.
    #[serde(rename = "retrieval_code")]
    pub retrieval_code: String,
}

/// Single report record returned from `12377.cn`.
#[derive(Debug, Clone, Serialize)]
pub struct ReportRecord {
    pub harm_type: String,
    pub retrieval_code: String,
    pub report_time: String,
    pub harm_url: String,
    pub result: String,
}

/// `POST /query` response body.
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub success: bool,
    pub total: usize,
    pub records: Vec<ReportRecord>,
    /// `null` on success.
    pub error: Option<String>,
}

impl QueryResponse {
    pub fn ok(records: Vec<ReportRecord>) -> Self {
        let total = records.len();
        Self {
            success: true,
            total,
            records,
            error: None,
        }
    }

    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            total: 0,
            records: Vec::new(),
            error: Some(msg.into()),
        }
    }
}

/// `GET /health` response body.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

impl Default for HealthResponse {
    fn default() -> Self {
        Self {
            status: "ok",
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}