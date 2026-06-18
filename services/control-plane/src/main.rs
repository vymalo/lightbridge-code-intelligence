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
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
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
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .route("/github/webhook", post(webhook::github_webhook))
        .route("/auth/verify", post(auth::verify))
        .with_state(state)
}

async fn liveness() -> &'static str {
    "ok"
}

/// Readiness fails closed when required configuration is missing, so a misconfigured pod
/// (e.g. no `GITHUB_WEBHOOK_SECRET`) is not handed traffic it would silently drop.
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    if state.github_webhook_secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "github webhook secret not configured",
        );
    }
    (StatusCode::OK, "ok")
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
    // Bind the raw string so hostnames (e.g. `localhost:8080`) resolve via `ToSocketAddrs`,
    // not only literal IP addresses.
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "control-plane listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
