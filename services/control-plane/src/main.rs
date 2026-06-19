//! Lightbridge control plane.
//!
//! The trust boundary of the system: it verifies GitHub webhooks, owns task/persistence
//! lifecycle, and acts as an OAuth2 **resource server** — validating OIDC access tokens (Keycloak
//! in dev) on protected routes. It does not issue tokens or store credentials; identity comes from
//! the validated JWT claims (see ADR-0014). This is a skeleton — persistence (cratestack/SQLx) and
//! task routing are intentionally stubbed; see docs/ and the ADRs.

mod jwt;
mod types;
mod webhook;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use tracing_subscriber::EnvFilter;

use jwt::JwtValidator;

#[derive(Clone)]
pub struct AppState {
    /// Secret used to verify GitHub webhook signatures (`X-Hub-Signature-256`).
    pub github_webhook_secret: Arc<String>,
    /// In-memory delivery-id dedup set. Production replaces this with the Postgres
    /// `github_deliveries` table (see docs/components-and-data-models.md).
    pub seen_deliveries: Arc<Mutex<HashSet<String>>>,
    /// OIDC token validator for protected routes. `None` when `OIDC_ISSUER` is unset, which makes
    /// protected routes fail closed (503) rather than silently accept unauthenticated traffic.
    pub jwt: Option<Arc<JwtValidator>>,
}

impl AppState {
    fn from_env() -> Self {
        Self {
            github_webhook_secret: Arc::new(
                std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            ),
            seen_deliveries: Arc::new(Mutex::new(HashSet::new())),
            jwt: jwt::from_env(),
        }
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .route("/github/webhook", post(webhook::github_webhook))
        .route("/me", get(jwt::me))
        .with_state(state)
}

async fn liveness() -> &'static str {
    "ok"
}

/// Readiness fails closed when required configuration is missing, so a misconfigured pod is not
/// handed traffic it would only reject: missing webhook secret, missing OIDC issuer, or an
/// unreachable IdP JWKS (which would 503 every protected request anyway).
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    if state.github_webhook_secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "github webhook secret not configured",
        );
    }
    match &state.jwt {
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "OIDC_ISSUER not configured",
        ),
        Some(validator) => match validator.warm().await {
            Ok(()) => (StatusCode::OK, "ok"),
            Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "oidc jwks unavailable"),
        },
    }
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
