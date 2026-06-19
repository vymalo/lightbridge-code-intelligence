//! Postgres persistence (hand-written SQLx; cratestack codegen deferred — ADR-0005).
//!
//! Runtime queries only (no compile-time `query!`), so the crate builds without a database. The
//! pool is optional: absent `DATABASE_URL` the control plane runs in a degraded, in-memory mode
//! (dev) and readiness reports it.

use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Connect to `DATABASE_URL` and run migrations. Returns `Ok(None)` when the URL is unset (dev).
/// **Fails fast** (`Err`) when the URL is set but the database is unreachable or migrations fail —
/// the process should exit so the orchestrator restarts it and retries, rather than running
/// permanently unready with no recovery path.
pub async fn connect_from_env() -> anyhow::Result<Option<PgPool>> {
    use anyhow::Context;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(error) => {
            return Err(anyhow::Error::from(error).context("failed to read DATABASE_URL"));
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .context("failed to connect to DATABASE_URL")?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("database migrations failed")?;
    tracing::info!("database connected and migrations applied");
    Ok(Some(pool))
}

/// Persist a GitHub delivery, using its `delivery_id` PRIMARY KEY for exactly-once handling.
/// Returns `true` if the delivery is new (inserted), `false` if it was already seen (duplicate).
pub async fn record_delivery(
    pool: &PgPool,
    delivery_id: &str,
    event_name: &str,
    payload: &Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO github_deliveries (delivery_id, event_name, payload_json) \
         VALUES ($1, $2, $3) ON CONFLICT (delivery_id) DO NOTHING",
    )
    .bind(delivery_id)
    .bind(event_name)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Liveness of the connection pool (used by readiness).
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

/// A task row as stored — one task run for the dashboard (ADR-0016). Serialized directly to the
/// `/tasks` API (timestamps as RFC 3339).
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TaskRow {
    pub id: Uuid,
    pub repository_id: i64,
    pub installation_id: i64,
    pub github_delivery_id: String,
    pub target_type: String,
    pub target_id: i64,
    pub command_text: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub status: String,
    pub priority: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

/// Fields needed to create a task from a webhook event (status starts at `received`).
pub struct NewTask {
    pub repository_id: i64,
    pub installation_id: i64,
    pub github_delivery_id: String,
    pub target_type: String,
    pub target_id: i64,
    pub command_text: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
}

/// Insert or update a repository by its GitHub id; returns the local `repositories.id`.
pub async fn upsert_repository(
    pool: &PgPool,
    github_repo_id: i64,
    owner: &str,
    name: &str,
    default_branch: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO repositories (github_repo_id, owner, name, default_branch) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (github_repo_id) DO UPDATE \
           SET owner = EXCLUDED.owner, name = EXCLUDED.name, default_branch = EXCLUDED.default_branch \
         RETURNING id",
    )
    .bind(github_repo_id)
    .bind(owner)
    .bind(name)
    .bind(default_branch)
    .fetch_one(pool)
    .await
}

/// Create a task; returns its generated id.
pub async fn create_task(pool: &PgPool, task: &NewTask) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO tasks (id, repository_id, installation_id, github_delivery_id, target_type, \
         target_id, command_text, base_sha, head_sha, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'received')",
    )
    .bind(id)
    .bind(task.repository_id)
    .bind(task.installation_id)
    .bind(&task.github_delivery_id)
    .bind(&task.target_type)
    .bind(task.target_id)
    .bind(&task.command_text)
    .bind(&task.base_sha)
    .bind(&task.head_sha)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Most recent tasks first (the dashboard run list).
pub async fn list_tasks(pool: &PgPool, limit: i64) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks ORDER BY created_at DESC LIMIT $1")
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// A single task by id.
pub async fn get_task(pool: &PgPool, id: Uuid) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}
