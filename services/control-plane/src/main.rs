//! Lightbridge control plane.
//!
//! The trust boundary of the system: it verifies GitHub webhooks, owns task/persistence
//! lifecycle, and exposes a standalone, portable authentication surface that the web app's
//! better-auth plugin verifies against. This is a skeleton — persistence (cratestack/SQLx)
//! and task routing are intentionally stubbed; see docs/ and the ADRs.

mod auth;
mod types;
mod webhook;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::routing::{get, post};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    /// Secret used to verify GitHub webhook signatures (`X-Hub-Signature-256`).
    pub github_webhook_secret: Arc<String>,
    /// In-memory delivery-id dedup set. Production replaces this with the Postgres
    /// `github_deliveries` table (see docs/components-and-data-models.md).
    pub seen_deliveries: Arc<Mutex<HashSet<String>>>,
}

impl AppState {
    fn from_env() -> Self {
        Self {
            github_webhook_secret: Arc::new(
                std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            ),
            seen_deliveries: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(health))
        .route("/github/webhook", post(webhook::github_webhook))
        .route("/auth/verify", post(auth::verify))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let state = AppState::from_env();
    let addr: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "control-plane listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
