//! Lightbridge control plane.
//!
//! The trust boundary of the system: it verifies GitHub webhooks, owns task/persistence
//! lifecycle, and acts as an OAuth2 **resource server** — validating OIDC access tokens (Keycloak
//! in dev) on protected routes. It does not issue tokens or store credentials; identity comes from
//! the validated JWT claims (see ADR-0014). Persistence is Postgres via hand-written SQLx
//! (cratestack codegen deferred — ADR-0005).
//!
//! # Module map
//!
//! Three concern groups (each its own submodule directory) sit on a few foundational modules:
//!
//! - [`http`] — the HTTP surface (`serve` role): [`webhook`](http::webhook) (GitHub webhooks, HMAC +
//!   delivery-id dedup), [`internal`](http::internal) (runner bootstrap/results API),
//!   [`admin`](http::admin) (dashboard admin), [`metrics`](http::metrics) (Prometheus `/metrics`).
//!   Routing + auth middleware live in this file.
//! - [`queue`] — queue & dispatch (`dispatcher` role): [`dispatcher`](queue::dispatcher) (claim +
//!   launch one Job per task), [`reaper`](queue::reaper) (Job GC + data-purge reconciler),
//!   [`lifecycle`](queue::lifecycle) (task state machine), [`tasks`](queue::tasks) (queue persistence).
//! - [`integrations`] — external systems the control plane owns credentials for:
//!   [`github`](integrations::github) (App auth + token mint + review write-back),
//!   [`k8s`](integrations::k8s) (Job manifests), [`neo4j`](integrations::neo4j) (graph writes).
//!
//! Foundational modules at the crate root: [`config`] (typed config load), [`db`] (pool +
//! migrations), [`jwt`] (OIDC token validation), [`types`] (shared domain types), [`review`]
//! (review validation).

mod http;
mod integrations;
mod queue;

// Foundational modules the groups above build on.
mod config;
mod db;
mod failure_notice;
mod jwt;
mod review;
mod types;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{DefaultBodyLimit, MatchedPath, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

use jwt::JwtValidator;
// Bring the grouped modules into scope under their bare names. `crate::` is required to
// disambiguate from the extern `http` / `metrics` crates pulled in by axum.
use crate::http::{admin, internal, metrics, webhook};
use crate::integrations::{github, k8s, neo4j};
use crate::queue::{dispatcher, tasks};

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
    /// Shared bearer for the internal runner API (`AGENT_RUNNER_TOKEN`). `None` disables those
    /// routes (they fail closed with 503) — the control plane injects the same value into each
    /// agent Job so the runner can authenticate back (see internal.rs / ADR-0017).
    pub runner_token: Option<Arc<String>>,
    /// Neo4j (Bolt) handle for the structural code graph (ADR-0019). `None` when `NEO4J_URI` is
    /// unset — the graph-ingest route then fails closed (503). Held here so the untrusted Job never
    /// gets Neo4j creds (it POSTs the graph through the internal API instead).
    pub neo4j: Option<Arc<neo4rs::Graph>>,
    /// Prometheus render handle backing `/metrics` (scraped by Alloy for the Operations dashboard).
    pub metrics: PrometheusHandle,
    /// Review-feedback config (PR reactions + outcome labels) from the file config's `review` section
    /// (else defaults). Held here so the webhook + internal handlers can react/label.
    pub review: Arc<config::ReviewSection>,
    /// The GitHub App's handle (e.g. `lightbridge-assistant`), from `GITHUB_APP_HANDLE`. A PR comment
    /// whose body starts with `@<handle>` triggers a re-review (the first review is automatic on PR
    /// open). Default `lightbridge-assistant`.
    pub app_handle: Arc<String>,
    /// Dotted claim path the caller's **permissions** list is read from (ADR-0023), from
    /// `PERMISSIONS_CLAIM`. Default `permissions`. Endpoints authorize on permissions, not roles.
    pub permissions_claim: Arc<String>,
}

impl AppState {
    async fn from_env() -> anyhow::Result<Self> {
        let metrics = metrics::install();
        // The file config is optional (ConfigMap-mounted): `review` drives PR feedback, `embeddings`
        // guards the vector column's dimension.
        let file_config = config::load_file_config().unwrap_or_else(|error| {
            tracing::error!(%error, "invalid control-plane config file; using defaults");
            None
        });
        let review = file_config
            .as_ref()
            .map(|f| f.review.clone())
            .unwrap_or_default();
        let embeddings = file_config
            .as_ref()
            .map(|f| f.embeddings.clone())
            .unwrap_or_default();
        let db = db::connect_from_env().await?;
        // Embedding-dimension safety (ADR-0018): if configured and the live column differs, either
        // reindex-from-scratch (when allowed) or fail loud — never silently mismatch.
        if let (Some(pool), Some(dimension)) = (db.as_ref(), embeddings.dimension) {
            db::reconcile_embedding_dimension(
                pool,
                dimension,
                embeddings.allow_reindex_on_dim_change,
            )
            .await?;
        }
        let neo4j = match neo4j::connect_from_env().await {
            Ok(handle) => handle.map(Arc::new),
            // A graph store outage shouldn't stop the control plane from serving everything else;
            // the graph-ingest route fails closed (503) when this is None.
            Err(error) => {
                tracing::error!(%error, "neo4j connection failed; graph ingestion disabled");
                None
            }
        };
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
            runner_token: std::env::var("AGENT_RUNNER_TOKEN")
                .ok()
                .filter(|token| !token.is_empty())
                .map(Arc::new),
            neo4j,
            metrics,
            review: Arc::new(review),
            app_handle: Arc::new(
                std::env::var("GITHUB_APP_HANDLE")
                    .ok()
                    .filter(|h| !h.trim().is_empty())
                    .unwrap_or_else(|| "lightbridge-assistant".to_string()),
            ),
            permissions_claim: Arc::new(
                std::env::var("PERMISSIONS_CLAIM")
                    .ok()
                    .filter(|c| !c.trim().is_empty())
                    .unwrap_or_else(|| "permissions".to_string()),
            ),
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

/// The database dimension of readiness. With a pool, readiness requires a successful ping; without
/// one (no `DATABASE_URL`), the pod is ready only under the dev opt-in — otherwise a misconfigured
/// production pod would silently run per-replica in-memory dedup yet report ready. The three
/// outcomes map to distinct `/readyz` messages so a database outage isn't mistaken for missing
/// config.
#[derive(Debug, PartialEq, Eq)]
enum DbReadiness {
    Ready,
    /// Pool present but the database did not answer (outage / network).
    Unreachable,
    /// No `DATABASE_URL` and no dev opt-in — fail closed.
    NotConfigured,
}

fn db_readiness(has_pool: bool, ping_ok: bool, allow_no_db: bool) -> DbReadiness {
    match (has_pool, ping_ok, allow_no_db) {
        (true, true, _) => DbReadiness::Ready,
        (true, false, _) => DbReadiness::Unreachable,
        (false, _, true) => DbReadiness::Ready,
        (false, _, false) => DbReadiness::NotConfigured,
    }
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(liveness))
        .route("/readyz", get(readiness))
        .route("/metrics", get(metrics_endpoint))
        .route("/github/webhook", post(webhook::github_webhook))
        .route("/me", get(jwt::me))
        .route("/tasks", get(tasks::list))
        .route("/tasks/{id}", get(tasks::get))
        .route("/tasks/{id}/review", get(tasks::get_review))
        .route("/tasks/{id}/transcript", get(tasks::get_transcript))
        .route("/tasks/{id}/feedback", get(tasks::get_feedback))
        .route("/tasks/{id}/cancel", post(tasks::cancel))
        .route("/repositories", get(tasks::list_repositories))
        // Admin API (approval gate, Epic #75) — gated by the `Admin` extractor (admin realm role).
        .route("/admin/repositories", get(admin::list_repositories))
        .route("/admin/repositories/{id}/approve", post(admin::approve))
        .route("/admin/repositories/{id}/deny", post(admin::deny))
        // Internal runner API (shared-bearer, not OIDC) — the agent Job's lifecycle callbacks.
        .route("/internal/tasks/{id}", get(internal::get_context))
        .route(
            "/internal/tasks/{id}/status",
            post(internal::set_status).get(internal::get_status),
        )
        // Chunk batches can be large: 32 chunks × 4096-dim embeddings as JSON ~1.6 MB plus
        // content. Raise the body limit to 16 MiB on this route only.
        .route(
            "/internal/tasks/{id}/chunks",
            post(internal::ingest_chunks).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        // The structural graph (Graphify → Neo4j, ADR-0019). A whole-repo graph.json can be large,
        // so raise the body limit here too.
        .route(
            "/internal/tasks/{id}/graph",
            post(internal::ingest_graph).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        // Retrieval for the MCP servers (slice 4): semantic search (pgvector) + structural queries
        // (Neo4j), each scoped server-side to the task's repo snapshot.
        .route("/internal/tasks/{id}/search", post(internal::search))
        .route(
            "/internal/tasks/{id}/graph/query",
            post(internal::graph_query),
        )
        // The agent run transcript (ADR-0034): the runner submits it at run end.
        .route(
            "/internal/tasks/{id}/transcript",
            post(internal::ingest_transcript),
        )
        // ADR-0037 mediated write actions: the native agent buffers findings/replies/summary, then
        // flushes them as one grouped review on finalize.
        .route(
            "/internal/tasks/{id}/review/inline",
            post(internal::add_review_comment),
        )
        .route(
            "/internal/tasks/{id}/review/inline/retract",
            post(internal::retract_inline),
        )
        .route(
            "/internal/tasks/{id}/review/inline/clear",
            post(internal::clear_inline),
        )
        .route(
            "/internal/tasks/{id}/review/comment",
            post(internal::add_review_reply),
        )
        .route(
            "/internal/tasks/{id}/review/summary",
            post(internal::set_review_summary),
        )
        .route(
            "/internal/tasks/{id}/review/finalize",
            post(internal::finalize_review),
        )
        .layer(axum::middleware::from_fn(track_http_metrics))
        .with_state(state)
}

async fn liveness() -> &'static str {
    "ok"
}

/// Prometheus text exposition for Alloy to scrape.
async fn metrics_endpoint(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
}

/// Middleware: record request count + latency by method, matched route, and status. Uses the
/// matched route (not the raw path) to keep label cardinality bounded.
///
/// Method and status are resolved to `&'static str` via a match so label allocation per request is
/// limited to a single `path.to_string()` inside `metrics::http_request`.
async fn track_http_metrics(req: Request, next: Next) -> Response {
    let method = match *req.method() {
        axum::http::Method::GET => "GET",
        axum::http::Method::POST => "POST",
        axum::http::Method::PUT => "PUT",
        axum::http::Method::DELETE => "DELETE",
        axum::http::Method::PATCH => "PATCH",
        axum::http::Method::HEAD => "HEAD",
        axum::http::Method::OPTIONS => "OPTIONS",
        axum::http::Method::CONNECT => "CONNECT",
        axum::http::Method::TRACE => "TRACE",
        _ => "UNKNOWN",
    };
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let start = Instant::now();
    let response = next.run(req).await;
    let status = match response.status().as_u16() {
        200 => "200",
        201 => "201",
        204 => "204",
        400 => "400",
        401 => "401",
        403 => "403",
        404 => "404",
        422 => "422",
        500 => "500",
        503 => "503",
        _ => "other",
    };
    let elapsed = start.elapsed().as_secs_f64();
    metrics::http_request(method, &path, status, elapsed);
    response
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
    // A configured database must be reachable. With no database, we report ready only when the dev
    // opt-in (`ALLOW_NO_DB`) is set — otherwise a misconfigured prod pod would silently dedup in
    // process memory across replicas. The two failure modes get distinct messages so an outage is
    // not mistaken for missing configuration.
    let ping_ok = match &state.db {
        Some(pool) => db::ping(pool).await.is_ok(),
        None => false,
    };
    match db_readiness(state.db.is_some(), ping_ok, state.allow_no_db) {
        DbReadiness::Ready => {}
        DbReadiness::Unreachable => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "database connection failed",
            )
        }
        DbReadiness::NotConfigured => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "database not configured (set DATABASE_URL; ALLOW_NO_DB=1 for dev only)",
            )
        }
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

    // One binary, several roles (RFC-0001): `serve` (HTTP) and `dispatcher` (queue consumer),
    // selected by the first CLI arg or `CONTROL_PLANE_ROLE`. Deployed as separate Deployments off
    // the same image so they scale independently. `scheduler` arrives in Phase 2.
    let role = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CONTROL_PLANE_ROLE").ok())
        .unwrap_or_else(|| "serve".to_string());

    let state = AppState::from_env().await?;
    match role.as_str() {
        "serve" => serve(state).await,
        "dispatcher" => run_dispatcher(state).await,
        "poller" => run_poller(state).await,
        other => anyhow::bail!("unknown role {other:?} (expected: serve | dispatcher | poller)"),
    }
}

/// The poller role (ADR-0035): a single replica that periodically reads reactions on the comments we
/// posted and reconciles them into `review_feedback`. Requires a database and — unlike serve — the
/// GitHub App key (to mint installation tokens); run as ONE replica so reactions aren't double-polled.
async fn run_poller(state: AppState) -> anyhow::Result<()> {
    let pool = state
        .db
        .clone()
        .ok_or_else(|| anyhow::anyhow!("poller requires DATABASE_URL"))?;
    let app = state.github.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "poller requires the GitHub App key (GITHUB_APP_ID + GITHUB_APP_PRIVATE_KEY)"
        )
    })?;
    spawn_metrics_server(state.metrics.clone());
    let interval = std::time::Duration::from_secs(
        std::env::var("POLLER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300),
    );
    let within_days = std::env::var("POLLER_WINDOW_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(14);
    queue::poller::run(pool, app, interval, within_days).await
}

/// The HTTP control surface (webhook ingress, `/tasks`, health, OIDC-protected routes).
async fn serve(state: AppState) -> anyhow::Result<()> {
    // Bind the raw string so hostnames (e.g. `localhost:8080`) resolve via `ToSocketAddrs`,
    // not only literal IP addresses.
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "control-plane listening");
    axum::serve(listener, app(state)).await?;
    Ok(())
}

/// The dispatcher role: consume the task queue and create one Kubernetes Job per task. Requires a
/// database (it is the queue) and a reachable cluster.
async fn run_dispatcher(state: AppState) -> anyhow::Result<()> {
    let pool = state
        .db
        .clone()
        .ok_or_else(|| anyhow::anyhow!("dispatcher requires DATABASE_URL"))?;
    // The dispatcher has no main HTTP server, so stand up a tiny one just for /metrics (+ health)
    // so Alloy can scrape it like the serve pods.
    spawn_metrics_server(state.metrics.clone());
    // Optional file config (ConfigMap-mounted); file-when-present-else-env for the agent-Job knobs
    // and dispatcher timings.
    let file_config = config::load_file_config()?;
    let launcher = k8s::KubeLauncher::resolve(file_config.as_ref().map(|f| &f.agent)).await?;
    let dispatcher_config =
        dispatcher::DispatcherConfig::from_file(file_config.as_ref().map(|f| &f.dispatcher));
    // The pod name is a natural, unique lease owner; fall back to a generic label off-cluster.
    let owner = std::env::var("HOSTNAME").unwrap_or_else(|_| "dispatcher".to_string());
    // Pass Neo4j so the dispatcher's loop can run the durable purge reconciler (graph + pg cleanup).
    dispatcher::run(
        pool,
        launcher,
        owner,
        dispatcher_config,
        state.neo4j.clone(),
    )
    .await
}

/// Serve `/metrics` (+ `/healthz`) on `METRICS_ADDR` for roles without a main HTTP server.
fn spawn_metrics_server(handle: PrometheusHandle) {
    let addr = std::env::var("METRICS_ADDR").unwrap_or_else(|_| "0.0.0.0:9090".to_string());
    tokio::spawn(async move {
        let router = Router::new().route("/healthz", get(liveness)).route(
            "/metrics",
            get(move || {
                let handle = handle.clone();
                async move {
                    (
                        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                        handle.render(),
                    )
                }
            }),
        );
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                tracing::info!(addr = %addr, "dispatcher metrics listening");
                if let Err(error) = axum::serve(listener, router).await {
                    tracing::error!(%error, "metrics server stopped");
                }
            }
            Err(error) => tracing::error!(%error, addr = %addr, "failed to bind metrics server"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_pool_readiness_follows_the_ping() {
        // A configured database must actually answer; a failed ping is reported as Unreachable
        // (distinct from missing config), and the dev opt-in never overrides it.
        assert_eq!(db_readiness(true, true, false), DbReadiness::Ready);
        assert_eq!(db_readiness(true, false, false), DbReadiness::Unreachable);
        assert_eq!(db_readiness(true, false, true), DbReadiness::Unreachable);
    }

    #[test]
    fn without_pool_requires_the_dev_opt_in() {
        // No DATABASE_URL: ready only when ALLOW_NO_DB is set — otherwise NotConfigured (fail
        // closed) so a misconfigured prod pod is never handed traffic it would dedup only in memory.
        assert_eq!(
            db_readiness(false, false, false),
            DbReadiness::NotConfigured
        );
        assert_eq!(db_readiness(false, true, false), DbReadiness::NotConfigured);
        assert_eq!(db_readiness(false, false, true), DbReadiness::Ready);
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
