//! HTTP client for `12377.cn`.
//!
//! Manages cookies manually (guestKey, JSESSIONID) instead of using
//! reqwest's built-in cookie store, because the cookie domain/jar needs
//! to work across both `www.12377.cn` and `new.12377.cn` hosts and we want
//! deterministic Set-Cookie parsing.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot_compat::Mutex;
use rand::Rng;
use reqwest::Client;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::config::Config;

/// Upstream API base URLs.
const WWW_URL: &str = "https://www.12377.cn";
const BS_URL: &str = "https://new.12377.cn";
const CAPTCHA_URL: &str = "https://new.12377.cn/rpapi/portal/captcha";
const QUERY_URL: &str = "https://new.12377.cn/rpapi/portal/report/get";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// upstream API response codes
pub const SUCCESS_CODE: i64 = 1000;
pub const CAPTCHA_ERR_CODE: i64 = 3104;

/// Errors that can come from `submit_query`. We need a custom type because
/// the orchestrator must distinguish a transport failure (retry) from a
/// clean "server said no" response.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error("server returned non-JSON body (status {status}): {body_prefix}")]
    BadJson { status: u16, body_prefix: String },
}

#[derive(Debug, Deserialize)]
pub struct QueryResp {
    /// 12377 returns this as either an integer or a string depending on
    /// the endpoint version — accept either.
    #[serde(deserialize_with = "deserialize_i64_lenient")]
    pub code: i64,
    #[serde(default, deserialize_with = "deserialize_i64_lenient")]
    pub total: i64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub data: Vec<RawRecord>,
}

/// Accept either a JSON number or a numeric string for an i64 field.
fn deserialize_i64_lenient<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let v = Value::deserialize(d)?;
    match v {
        Value::Number(n) => n.as_i64().ok_or_else(|| serde::de::Error::custom("not i64")),
        Value::String(s) => s.parse::<i64>().map_err(serde::de::Error::custom),
        Value::Null => Ok(0),
        _ => Err(serde::de::Error::custom("expected number or numeric string")),
    }
}

#[derive(Debug, Deserialize)]
pub struct RawRecord {
    #[serde(default, rename = "harmTypeTopStr")]
    pub harm_type_top_str: String,
    #[serde(default, rename = "retrievalCode")]
    pub retrieval_code: String,
    #[serde(default, rename = "reportTimeStrDay")]
    pub report_time_str_day: String,
    #[serde(default, rename = "harmUrl")]
    pub harm_url: String,
    #[serde(default, rename = "quickReply")]
    pub quick_reply: String,
}

/// Thread-safe cookie jar.
#[derive(Default)]
struct CookieJar {
    inner: Mutex<Vec<(String, String)>>, // (name, value)
}

impl CookieJar {
    fn add(&self, name: &str, value: &str) {
        let mut g = self.inner.lock().expect("cookie jar poisoned");
        if let Some(slot) = g.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value.to_string();
        } else {
            g.push((name.to_string(), value.to_string()));
        }
    }

    fn header_value(&self) -> String {
        let g = self.inner.lock().expect("cookie jar poisoned");
        g.iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Pre-built client.
pub struct HttpClient {
    http: Client,
    cookies: Arc<CookieJar>,
    captcha_timeout: u64,
    query_timeout: u64,
}

impl HttpClient {
    /// Build a new client and bootstrap the session by hitting the homepage.
    pub async fn new(cfg: &Config) -> Result<Self, reqwest::Error> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .gzip(true)
            .build()?;

        let cookies = Arc::new(CookieJar::default());

        let me = Self {
            http,
            cookies,
            captcha_timeout: cfg.captcha_timeout_secs,
            query_timeout: cfg.query_timeout_secs,
        };

        me.bootstrap().await;
        Ok(me)
    }

    /// Visit homepage to populate Referer chain, then inject `guestKey`.
    async fn bootstrap(&self) {
        match self
            .http
            .get(format!("{WWW_URL}/jbcx.html?tab=6"))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
        {
            Ok(r) => debug!(status = r.status().as_u16(), "homepage ok"),
            Err(e) => warn!(error = %e, "homepage visit failed (ignored)"),
        }
        let guest_key = generate_guest_key();
        self.cookies.add("guestKey", &guest_key);
        info!(guest_key = %guest_key, "session bootstrapped");
    }

    /// GET the captcha PNG; populates JSESSIONID cookie.
    pub async fn fetch_captcha(&self) -> Result<Vec<u8>, reqwest::Error> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let url = format!("{CAPTCHA_URL}?{ts}");

        let resp = self
            .http
            .get(&url)
            .header("Referer", format!("{WWW_URL}/jbcx.html?tab=6"))
            .header(
                "Accept",
                "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
            )
            .header("Cookie", self.cookies.header_value())
            .timeout(std::time::Duration::from_secs(self.captcha_timeout))
            .send()
            .await?;
        resp.error_for_status_ref()?;

        // Parse Set-Cookie for JSESSIONID
        if let Some(set_cookie) = resp.headers().get(reqwest::header::SET_COOKIE) {
            if let Ok(s) = set_cookie.to_str() {
                if let Some(jid) = s.split("JSESSIONID=").nth(1) {
                    let jid = jid.split(';').next().unwrap_or("");
                    if !jid.is_empty() {
                        self.cookies.add("JSESSIONID", jid);
                        debug!(jsid_prefix = %&jid[..jid.len().min(20)], "JSESSIONID captured");
                    }
                }
            }
        }

        let bytes = resp.bytes().await?.to_vec();
        debug!(len = bytes.len(), "captcha fetched");
        Ok(bytes)
    }

    /// POST the query with captcha answer.
    pub async fn submit_query(
        &self,
        retrieval_code: &str,
        verify_code: &str,
    ) -> Result<QueryResp, ClientError> {
        let form = [
            ("retrievalCode", retrieval_code),
            ("verifyCode", verify_code),
            ("pageSize", "1000"),
        ];

        let resp = self
            .http
            .post(QUERY_URL)
            .form(&form)
            .header("Referer", format!("{WWW_URL}/jbcx.html?tab=6"))
            .header("Origin", WWW_URL)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Cookie", self.cookies.header_value())
            .timeout(std::time::Duration::from_secs(self.query_timeout))
            .send()
            .await?;
        resp.error_for_status_ref()?;

        let resp_status = resp.status().as_u16();

        // Capture any new cookies from this response too.
        if let Some(set_cookie) = resp.headers().get(reqwest::header::SET_COOKIE) {
            if let Ok(s) = set_cookie.to_str() {
                for part in s.split(';') {
                    if let Some(eq) = part.find('=') {
                        let name = part[..eq].trim();
                        let value = part[eq + 1..].trim();
                        if !name.is_empty() && !value.is_empty() {
                            self.cookies.add(name, value);
                        }
                    }
                }
            }
        }

        // Read body once, then parse — so we can log it on failure.
        let resp_body = resp.bytes().await?.to_vec();
        match serde_json::from_slice::<QueryResp>(&resp_body) {
            Ok(parsed) => {
                debug!(
                    code = parsed.code,
                    total = parsed.total,
                    "query response received"
                );
                Ok(parsed)
            }
            Err(e) => {
                let body_str = String::from_utf8_lossy(&resp_body);
                let prefix: String = body_str.chars().take(200).collect();
                warn!(
                    error = %e,
                    status = resp_status,
                    body_prefix = %prefix,
                    "JSON decode failed on query response"
                );
                Err(ClientError::BadJson { status: resp_status, body_prefix: prefix })
            }
        }
    }
}

/// `guestKey = YYYYMMDDHHmmss + 6 random [A-Z0-9]` — matches the Python helper.
fn generate_guest_key() -> String {
    use chrono::Local;
    let now = Local::now().format("%Y%m%d%H%M%S").to_string();
    const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let suffix: String = (0..6)
        .map(|_| CHARS[rand::thread_rng().gen_range(0..CHARS.len())] as char)
        .collect();
    format!("{now}{suffix}")
}

mod parking_lot_compat {
    // Lightweight std-only Mutex shim — parking_lot is heavy and we already
    // have std::sync::Mutex. This re-exports as `Mutex` to keep code tidy.
    pub use std::sync::Mutex;
}