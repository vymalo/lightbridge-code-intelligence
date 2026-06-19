//! Lightbridge control plane.
//!
//! The trust boundary of the system: it verifies GitHub webhooks, owns task/persistence
//! lifecycle, and acts as an OAuth2 **resource server** — validating OIDC access tokens (Keycloak
//! in dev) on protected routes. It does not issue tokens or store credentials; identity comes from
//! the validated JWT claims (see ADR-0014). Persistence is Postgres via hand-written SQLx
//! (cratestack codegen deferred — ADR-0005).

mod db;
mod github;
mod jwt;
mod tasks;
mod types;
mod webhook;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

use jwt::JwtValidator;

#[derive(Clone)]
pub struct AppState {
    /// Secret used to verify GitHub webhook signatures (`X-Hub-Signature-256`).
    pub github_webhook_secret: Arc<String>,
    /// In-memory delivery-id dedup set — the fallback when no database is configured (dev). With a
    /// pool, the webhook dedups on the `github_deliveries` PRIMARY KEY instead.
    pub seen_deliveries: Arc<Mutex<HashSet<String>>>,
    /// OIDC token validator for protected routes. `None` when `OIDC_ISSUER` is unset (fails closed).
    pub jwt: Option<Arc<JwtValidator>>,
    /// Postgres pool. `None` when `DATABASE_URL` is unset (dev) or the connection failed.
    pub db: Option<PgPool>,
    /// GitHub App auth (App JWT → installation tokens). `None` when the App env is unset.
    pub github: Option<github::GithubApp>,
}

impl AppState {
    async fn from_env() -> anyhow::Result<Self> {
        let db = db::connect_from_env().await?;
        Ok(Self {
            github_webhook_secret: Arc::new(
                std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            ),
            seen_deliveries: Arc::new(Mutex::new(HashSet::new())),
            jwt: jwt::from_env(),
            db,
            github: github::GithubApp::from_env(),
        })
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .route("/github/webhook", post(webhook::github_webhook))
        .route("/me", get(jwt::me))
        .route("/tasks", get(tasks::list))
        .route("/tasks/{id}", get(tasks::get))
        .with_state(state)
}

async fn liveness() -> &'static str {
    "ok"
}

/// Readiness fails closed when required configuration is missing or a dependency is unreachable, so
/// a misconfigured pod is not handed traffic it would only reject: missing webhook secret, missing
/// OIDC issuer / unreachable JWKS, or a configured-but-unreachable database.
async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    if state.github_webhook_secret.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "github webhook secret not configured",
        );
    }
    match &state.jwt {
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "OIDC_ISSUER not configured",
            )
        }
        Some(validator) => {
            if validator.warm().await.is_err() {
                return (StatusCode::SERVICE_UNAVAILABLE, "oidc jwks unavailable");
            }
        }
    }
    // A configured database must be reachable; if `DATABASE_URL` is unset (dev) we run degraded.
    let db_ok = match &state.db {
        Some(pool) => db::ping(pool).await.is_ok(),
        None => std::env::var("DATABASE_URL").is_err(),
    };
    if !db_ok {
        return (StatusCode::SERVICE_UNAVAILABLE, "database unavailable");
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

    let state = AppState::from_env().await?;
    // Bind the raw string so hostnames (e.g. `localhost:8080`) resolve via `ToSocketAddrs`,
    // not only literal IP addresses.
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "control-plane listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}
