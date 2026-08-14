//! Runtime configuration loaded from environment variables.

use std::env;

/// Process configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub max_retries: u32,
    pub captcha_timeout_secs: u64,
    pub query_timeout_secs: u64,
}

impl Config {
    /// Load configuration from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8000),
            // 10 retries gives a fresh captcha each round; the OCR is ~70%
            // per attempt so 10 attempts → ~99.99% effective success.
            max_retries: env::var("MAX_RETRIES")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(10),
            captcha_timeout_secs: env::var("CAPTCHA_TIMEOUT")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(15),
            query_timeout_secs: env::var("QUERY_TIMEOUT")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(30),
        }
    }
}