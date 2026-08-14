//! Query orchestration — drives the captcha-solve loop and parses results.

use tracing::{info, warn};

use crate::captcha;
use crate::client::{ClientError, HttpClient, QueryResp, RawRecord, CAPTCHA_ERR_CODE, SUCCESS_CODE};
use crate::config::Config;
use crate::error::ApiError;
use crate::models::{QueryResponse, ReportRecord};

/// Execute a full query: fetch captcha → solve → submit, retrying on captcha
/// errors up to `cfg.max_retries` times.
pub async fn do_query(cfg: &Config, retrieval_code: &str) -> Result<QueryResponse, ApiError> {
    let client = HttpClient::new(cfg).await?;

    for attempt in 1..=cfg.max_retries {
        info!(attempt, max = cfg.max_retries, "query attempt");

        // 1. captcha
        let img = match client.fetch_captcha().await {
            Ok(b) => b,
            Err(e) => {
                warn!(attempt, error = %e, "captcha fetch failed");
                if attempt < cfg.max_retries {
                    continue;
                }
                return Err(ApiError::Network(e));
            }
        };

        // 2. solve
        let answer = match captcha::recognize(&img) {
            Some(a) => a,
            None => {
                warn!(attempt, "captcha recognition failed");
                continue;
            }
        };
        info!(answer = %answer, "captcha solved");

        // 3. submit
        let resp = match client.submit_query(retrieval_code, &answer).await {
            Ok(r) => r,
            Err(ClientError::BadJson { status, body_prefix }) => {
                // Server returned non-JSON (rate-limit / WAF / maintenance page).
                // Retry with a fresh captcha — but don't burn more than 2 attempts
                // on persistent non-JSON responses.
                warn!(attempt, status, body_prefix, "submit returned non-JSON; retrying");
                if attempt < cfg.max_retries {
                    continue;
                }
                return Err(ApiError::Internal(format!(
                    "upstream returned non-JSON after {} attempts (last status={})",
                    cfg.max_retries, status
                )));
            }
            Err(ClientError::Network(e)) => {
                warn!(attempt, error = %e, "submit failed (network)");
                if attempt < cfg.max_retries {
                    continue;
                }
                return Err(ApiError::Network(e));
            }
        };

        // 4. dispatch
        match dispatch(resp) {
            DispatchResult::Records(rs) => return Ok(QueryResponse::ok(rs)),
            DispatchResult::CaptchaError => {
                warn!(attempt, "server rejected captcha, retrying");
                continue;
            }
            DispatchResult::BadCode(code, msg) => {
                warn!(code, msg, "server returned unexpected code");
                return Ok(QueryResponse::fail(format!(
                    "upstream returned code {code}: {msg}"
                )));
            }
        }
    }

    Ok(QueryResponse::fail(format!(
        "captcha recognition failed after {} attempts",
        cfg.max_retries
    )))
}

enum DispatchResult {
    Records(Vec<ReportRecord>),
    CaptchaError,
    BadCode(i64, String),
}

fn dispatch(resp: QueryResp) -> DispatchResult {
    if resp.code == SUCCESS_CODE {
        let records: Vec<ReportRecord> = resp.data.into_iter().map(to_record).collect();
        return DispatchResult::Records(records);
    }
    if resp.code == CAPTCHA_ERR_CODE {
        return DispatchResult::CaptchaError;
    }
    DispatchResult::BadCode(resp.code, resp.message)
}

fn to_record(r: RawRecord) -> ReportRecord {
    ReportRecord {
        harm_type: r.harm_type_top_str,
        retrieval_code: r.retrieval_code,
        report_time: r.report_time_str_day,
        harm_url: r.harm_url,
        result: r.quick_reply,
    }
}