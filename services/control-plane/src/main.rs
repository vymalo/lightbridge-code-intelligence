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
    /// Dev-only opt-in (`ALLOW_NO_DB=1`) to run without a database. Without it, a pod that has no
    /// `DATABASE_URL` fails readiness instead of silently dedup'ing in process memory — which would
    /// reintroduce the multi-replica duplicate-task bug (RFC-0001 Phase 0).
    pub allow_no_db: bool,
    /// GitHub App auth (App JWT → installation tokens). `None` when the App env is unset.
    pub github: Option<github::GithubApp>,
}

impl AppState {
    async fn from_env() -> anyhow::Result<Self> {
        let db = db::connect_from_env().await?;
        let allow_no_db = env_flag("ALLOW_NO_DB");
        if db.is_none() {
            if allow_no_db {
                tracing::warn!(
                    "running WITHOUT a database (ALLOW_NO_DB): in-memory dedup, single-replica only — dev use"
                );
            } else {
                tracing::error!(
                    "DATABASE_URL is not set and ALLOW_NO_DB is unset: the pod will fail readiness. \
                     Set DATABASE_URL in production, or ALLOW_NO_DB=1 for local dev."
                );
            }
        }
        Ok(Self {
            github_webhook_secret: Arc::new(
                std::env::var("GITHUB_WEBHOOK_SECRET").unwrap_or_default(),
            ),
            seen_deliveries: Arc::new(Mutex::new(HashSet::new())),
            jwt: jwt::from_env(),
            db,
            allow_no_db,
            github: github::GithubApp::from_env(),
        })
    }
}

/// A boolean env flag that is true for `1`/`true`/`yes` (case-insensitive), false otherwise.
fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

/// Whether the database dimension of readiness passes. With a pool, requires a successful ping.
/// Without a pool (no `DATABASE_URL`), only ready when the dev opt-in is set — otherwise a
/// misconfigured production pod would silently run per-replica in-memory dedup yet report ready.
fn db_dimension_ready(has_pool: bool, ping_ok: bool, allow_no_db: bool) -> bool {
    if has_pool {
        ping_ok
    } else {
        allow_no_db
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
    // A configured database must be reachable. With no database, we report ready only when the
    // dev opt-in (`ALLOW_NO_DB`) is set — otherwise a misconfigured prod pod would fail closed
    // rather than silently dedup in process memory across replicas.
    let db_ok = match &state.db {
        Some(pool) => db_dimension_ready(true, db::ping(pool).await.is_ok(), state.allow_no_db),
        None => db_dimension_ready(false, false, state.allow_no_db),
    };
    if !db_ok {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "database not configured (set DATABASE_URL; ALLOW_NO_DB=1 for dev only)",
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

    let state = AppState::from_env().await?;
    // Bind the raw string so hostnames (e.g. `localhost:8080`) resolve via `ToSocketAddrs`,
    // not only literal IP addresses.
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "control-plane listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_pool_readiness_follows_the_ping() {
        // A configured database must actually answer; the dev opt-in never overrides a failed ping.
        assert!(db_dimension_ready(true, true, false));
        assert!(!db_dimension_ready(true, false, false));
        assert!(!db_dimension_ready(true, false, true));
    }

    #[test]
    fn without_pool_requires_the_dev_opt_in() {
        // No DATABASE_URL: ready only when ALLOW_NO_DB is set — otherwise fail closed so a
        // misconfigured prod pod is never handed traffic it would dedup only in process memory.
        assert!(!db_dimension_ready(false, false, false));
        assert!(db_dimension_ready(false, false, true));
    }

    #[test]
    fn env_flag_parses_truthy_values_only() {
        for truthy in ["1", "true", "TRUE", "Yes"] {
            std::env::set_var("CP_TEST_FLAG", truthy);
            assert!(env_flag("CP_TEST_FLAG"), "{truthy} should be truthy");
        }
        for falsy in ["0", "false", "no", "", "off"] {
            std::env::set_var("CP_TEST_FLAG", falsy);
            assert!(!env_flag("CP_TEST_FLAG"), "{falsy} should be falsy");
        }
        std::env::remove_var("CP_TEST_FLAG");
        assert!(!env_flag("CP_TEST_FLAG"));
    }
}
