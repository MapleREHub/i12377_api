//! 12377 举报查询 API — Rust port.
//!
//! Single-binary service exposing:
//! - `GET  /health` — liveness probe
//! - `POST /query`  — query a single report (JSON body: `{"retrieval_code": "..."}`)
//!
//! Run:
//! ```text
//! cargo run --release
//! HOST=0.0.0.0 PORT=8000 RUST_LOG=info cargo run --release
//! ```

mod captcha;
mod client;
mod config;
mod error;
mod models;
mod orchestrator;
mod routes;

use std::net::SocketAddr;

use axum::{routing::{get, post}, Router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- tracing ----
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // ---- config ----
    let cfg = Config::from_env();
    tracing::info!(
        host = %cfg.host,
        port = cfg.port,
        max_retries = cfg.max_retries,
        "starting i12377_api"
    );

    // ---- router ----
    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/query", post(routes::query))
        .with_state(cfg.clone());

    // ---- bind ----
    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");

    // ---- serve ----
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Wait for SIGINT / SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}