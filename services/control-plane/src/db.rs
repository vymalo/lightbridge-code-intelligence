//! Postgres persistence (hand-written SQLx; cratestack codegen deferred — ADR-0005).
//!
//! Runtime queries only (no compile-time `query!`), so the crate builds without a database. The
//! pool is optional: absent `DATABASE_URL` the control plane runs in a degraded, in-memory mode
//! (dev) and readiness reports it.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Postgres `LISTEN`/`NOTIFY` channel the dispatcher waits on; `create_task` notifies it on enqueue
/// so a dispatcher reacts immediately instead of waiting for its poll fallback.
pub const TASK_QUEUED_CHANNEL: &str = "task_queued";

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
/// `/tasks` API (timestamps as RFC 3339). The `repo_*` fields are joined from `repositories` so the
/// dashboard can show a human repo name + branch without a second round-trip (LEFT JOIN, so they're
/// `None` for the rare orphaned row).
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
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_default_branch: Option<String>,
}

/// `SELECT` projection shared by the list and detail queries: every `tasks` column plus the joined
/// repository identity, aliased to the `repo_*` fields of [`TaskRow`].
const TASK_SELECT: &str = "SELECT t.*, r.owner AS repo_owner, r.name AS repo_name, \
     r.default_branch AS repo_default_branch \
     FROM tasks t LEFT JOIN repositories r ON r.id = t.repository_id";

/// Fields needed to create a task from a webhook event.
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

/// A task claimed by the dispatcher for execution (the subset needed to launch its Job).
#[derive(Debug, sqlx::FromRow)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub repository_id: i64,
    pub installation_id: i64,
    pub target_type: String,
    pub target_id: i64,
    pub command_text: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub attempts: i32,
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

/// Enqueue a task idempotently. Returns the new task id, or `None` when an equivalent task already
/// exists — GitHub can deliver several events for one PR head (e.g. `opened` then `synchronize`),
/// and the `tasks_idempotency_idx` unique index collapses those to a single `queued` task. On a real
/// insert, notifies [`TASK_QUEUED_CHANNEL`] so a listening dispatcher reacts immediately.
pub async fn create_task(pool: &PgPool, task: &NewTask) -> Result<Option<Uuid>, sqlx::Error> {
    let id = Uuid::new_v4();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO tasks (id, repository_id, installation_id, github_delivery_id, target_type, \
         target_id, command_text, base_sha, head_sha, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'queued') \
         ON CONFLICT (repository_id, target_type, target_id, command_text, head_sha, run_epoch) \
         DO NOTHING \
         RETURNING id",
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
    .fetch_optional(pool)
    .await?;

    if let Some(new_id) = inserted {
        // Wake a listening dispatcher; harmless if none is connected (the dispatcher also polls).
        let _ = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(TASK_QUEUED_CHANNEL)
            .bind(new_id.to_string())
            .execute(pool)
            .await;
    }
    Ok(inserted)
}

/// Atomically claim the next due `queued` task and take a short dispatch lease. `FOR UPDATE SKIP
/// LOCKED` guarantees that concurrent dispatcher replicas never claim the same row. Returns `None`
/// when nothing is due. (Lease expiry is reaped by the scheduler in RFC-0001 Phase 2.)
pub async fn claim_next_task(
    pool: &PgPool,
    owner: &str,
    lease: Duration,
) -> Result<Option<ClaimedTask>, sqlx::Error> {
    sqlx::query_as::<_, ClaimedTask>(
        "UPDATE tasks \
         SET status = 'running', attempts = attempts + 1, started_at = now(), \
             lease_owner = $1, lease_expires_at = now() + ($2 * interval '1 second') \
         WHERE id = ( \
           SELECT id FROM tasks \
           WHERE status = 'queued' AND run_after <= now() \
           ORDER BY priority DESC, created_at \
           FOR UPDATE SKIP LOCKED \
           LIMIT 1 \
         ) \
         RETURNING id, repository_id, installation_id, target_type, target_id, command_text, \
                   base_sha, head_sha, attempts",
    )
    .bind(owner)
    .bind(lease.as_secs_f64())
    .fetch_optional(pool)
    .await
}

/// Record the Kubernetes Job created for a dispatched task.
pub async fn set_task_job(pool: &PgPool, id: Uuid, job_name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET job_name = $2 WHERE id = $1")
        .bind(id)
        .bind(job_name)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Return a claimed task to the queue with a backoff delay (e.g. Job creation failed). Clears the
/// lease so another dispatcher can pick it up after `run_after`.
pub async fn release_task(pool: &PgPool, id: Uuid, backoff: Duration) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tasks \
         SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL, started_at = NULL, \
             run_after = now() + ($2 * interval '1 second') \
         WHERE id = $1",
    )
    .bind(id)
    .bind(backoff.as_secs_f64())
    .execute(pool)
    .await
    .map(|_| ())
}

/// Most recent tasks first (the dashboard run list).
pub async fn list_tasks(pool: &PgPool, limit: i64) -> Result<Vec<TaskRow>, sqlx::Error> {
    let sql = format!("{TASK_SELECT} ORDER BY t.created_at DESC LIMIT $1");
    sqlx::query_as::<_, TaskRow>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// A single task by id.
pub async fn get_task(pool: &PgPool, id: Uuid) -> Result<Option<TaskRow>, sqlx::Error> {
    let sql = format!("{TASK_SELECT} WHERE t.id = $1");
    sqlx::query_as::<_, TaskRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// A connected repository for the dashboard's Repositories view (ADR-0016), with a small activity
/// summary (run count + most-recent run) derived from `tasks`. RepoIndex health is not joined yet —
/// the indexer that populates `repo_index` is a later step in the Code product epic.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RepositoryRow {
    pub id: i64,
    pub github_repo_id: i64,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub active: bool,
    pub task_count: i64,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_task_at: Option<OffsetDateTime>,
}

/// All connected repositories, most-recently-active first. Aggregates each repo's task activity in
/// one query so the Repositories list needs no per-row round-trip.
pub async fn list_repositories(pool: &PgPool) -> Result<Vec<RepositoryRow>, sqlx::Error> {
    sqlx::query_as::<_, RepositoryRow>(
        "SELECT r.id, r.github_repo_id, r.owner, r.name, r.default_branch, r.active, \
           COUNT(t.id) AS task_count, MAX(t.created_at) AS last_task_at \
         FROM repositories r LEFT JOIN tasks t ON t.repository_id = r.id \
         GROUP BY r.id \
         ORDER BY last_task_at DESC NULLS LAST, r.owner, r.name",
    )
    .fetch_all(pool)
    .await
}

/// Everything the agent runner needs to act on a task, joined with its repository identity. Served
/// by the internal runner API (`GET /internal/tasks/{id}`) so the runner never holds the GitHub App
/// key — it receives repo coordinates here and a freshly-minted installation token alongside (the
/// control plane mints it; see `internal.rs`). `installation_id` is kept server-side for that.
#[derive(Debug, sqlx::FromRow)]
pub struct TaskContextRow {
    pub id: Uuid,
    pub repository_id: i64,
    pub installation_id: i64,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub target_type: String,
    pub target_id: i64,
    pub command_text: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
}

/// Load a task's execution context, or `None` if no such task exists. INNER JOIN on `repositories`:
/// a task always references a repository (FK), so a missing row means a bad/expired id.
pub async fn get_task_context(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<TaskContextRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskContextRow>(
        "SELECT t.id, t.repository_id, t.installation_id, r.owner, r.name, r.default_branch, \
                t.target_type, t.target_id, t.command_text, t.base_sha, t.head_sha \
         FROM tasks t JOIN repositories r ON r.id = t.repository_id \
         WHERE t.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Statuses the runner is allowed to report. Transitioning into a terminal one stamps
/// `completed_at` and releases the lease; `running` (re)stamps `started_at`. Anything else is
/// rejected by the handler before reaching here.
pub fn is_runner_reportable_status(status: &str) -> bool {
    matches!(
        status,
        "running" | "posting_result" | "succeeded" | "failed" | "timed_out" | "cancelled"
    )
}

/// Apply a runner-reported status transition. Terminal states (`succeeded`/`failed`/`timed_out`/
/// `cancelled`) stamp `completed_at` and clear the dispatcher lease so the reaper (Phase 2) won't
/// reclaim a finished task; `running` stamps `started_at` if unset. Returns `false` if no task
/// matched the id. The caller validates `status` with [`is_runner_reportable_status`] first.
pub async fn set_task_status(pool: &PgPool, id: Uuid, status: &str) -> Result<bool, sqlx::Error> {
    let terminal = matches!(status, "succeeded" | "failed" | "timed_out" | "cancelled");
    let result = sqlx::query(
        "UPDATE tasks SET \
             status = $2, \
             started_at = CASE WHEN $2 = 'running' THEN COALESCE(started_at, now()) ELSE started_at END, \
             completed_at = CASE WHEN $3 THEN now() ELSE completed_at END, \
             lease_owner = CASE WHEN $3 THEN NULL ELSE lease_owner END, \
             lease_expires_at = CASE WHEN $3 THEN NULL ELSE lease_expires_at END \
         WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(terminal)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Integration tests: `#[sqlx::test]` provisions a fresh database, runs the migrations, and hands
    // us a pool. Requires a reachable Postgres via `DATABASE_URL` (see `compose.yaml`); skipped when
    // none is configured (CI builds images but runs no Rust test job today).

    /// The dedup contract that lets the control plane run multiple replicas: the `delivery_id`
    /// PRIMARY KEY + `ON CONFLICT DO NOTHING` means a replayed GitHub delivery is detected as a
    /// duplicate (GitHub delivers at least once), and the row is written exactly once.
    #[sqlx::test]
    async fn record_delivery_dedupes_on_delivery_id(pool: PgPool) {
        let payload = json!({ "action": "opened" });

        let first = record_delivery(&pool, "delivery-abc", "pull_request", &payload)
            .await
            .unwrap();
        assert!(first, "first delivery is new");

        let replay = record_delivery(&pool, "delivery-abc", "pull_request", &payload)
            .await
            .unwrap();
        assert!(!replay, "replayed delivery id is a duplicate");

        let other = record_delivery(&pool, "delivery-xyz", "push", &payload)
            .await
            .unwrap();
        assert!(other, "a different delivery id is independent");

        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM github_deliveries WHERE delivery_id = $1")
                .bind("delivery-abc")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "the replayed delivery is stored exactly once");
    }

    /// Seed the FK rows a task needs (one repository + one delivery); returns the repository id.
    async fn seed(pool: &PgPool) -> i64 {
        let repo_id = upsert_repository(pool, 1, "octo", "repo", "main")
            .await
            .unwrap();
        record_delivery(pool, "d1", "pull_request", &json!({}))
            .await
            .unwrap();
        repo_id
    }

    fn pr_task(repository_id: i64, head: &str) -> NewTask {
        NewTask {
            repository_id,
            installation_id: 99,
            github_delivery_id: "d1".to_string(),
            target_type: "pull_request".to_string(),
            target_id: 7,
            command_text: "review".to_string(),
            base_sha: Some("base".to_string()),
            head_sha: Some(head.to_string()),
        }
    }

    /// Task creation is idempotent on (repo, target, command, head SHA): a second `pull_request`
    /// event for the same head (e.g. `opened` then `synchronize`) does not create a duplicate task,
    /// but a new head SHA does.
    #[sqlx::test]
    async fn create_task_is_idempotent_on_target_and_head(pool: PgPool) {
        let repo_id = seed(&pool).await;

        let first = create_task(&pool, &pr_task(repo_id, "head1"))
            .await
            .unwrap();
        assert!(first.is_some(), "first task is created");

        let dup = create_task(&pool, &pr_task(repo_id, "head1"))
            .await
            .unwrap();
        assert!(dup.is_none(), "equivalent task is deduped");

        let new_head = create_task(&pool, &pr_task(repo_id, "head2"))
            .await
            .unwrap();
        assert!(new_head.is_some(), "a new head SHA is a new task");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    /// The dispatcher claim takes exactly one queued task and leaves none for the next claim — the
    /// `SKIP LOCKED` guard that lets dispatcher replicas run concurrently without double-claiming.
    #[sqlx::test]
    async fn claim_next_task_takes_one_queued_task(pool: PgPool) {
        let repo_id = seed(&pool).await;
        create_task(&pool, &pr_task(repo_id, "head1"))
            .await
            .unwrap()
            .unwrap();

        let claimed = claim_next_task(&pool, "owner-a", Duration::from_secs(60))
            .await
            .unwrap();
        let claimed = claimed.expect("a queued task is claimed");
        assert_eq!(claimed.attempts, 1, "claim increments attempts");
        assert_eq!(claimed.command_text, "review");

        let none = claim_next_task(&pool, "owner-b", Duration::from_secs(60))
            .await
            .unwrap();
        assert!(none.is_none(), "the claimed task is no longer queued");
    }

    /// A released task returns to the queue and can be claimed again (Job-launch failure path).
    #[sqlx::test]
    async fn release_task_requeues_for_another_claim(pool: PgPool) {
        let repo_id = seed(&pool).await;
        create_task(&pool, &pr_task(repo_id, "head1"))
            .await
            .unwrap()
            .unwrap();

        let first = claim_next_task(&pool, "owner-a", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        // Zero backoff so it is immediately due again.
        release_task(&pool, first.id, Duration::from_secs(0))
            .await
            .unwrap();

        // Releasing clears started_at so the dashboard doesn't show a queued task as already running.
        let started_at: Option<OffsetDateTime> =
            sqlx::query_scalar("SELECT started_at FROM tasks WHERE id = $1")
                .bind(first.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(started_at.is_none(), "release clears started_at");

        let second = claim_next_task(&pool, "owner-a", Duration::from_secs(60))
            .await
            .unwrap()
            .expect("released task is claimable again");
        assert_eq!(second.id, first.id);
        assert_eq!(second.attempts, 2, "the re-claim counts as another attempt");
    }

    /// `list_repositories` aggregates run activity (it's runtime SQL, so this is the only place the
    /// GROUP BY/JOIN is exercised): a repo with two tasks reports `task_count = 2`, an idle repo
    /// reports `0` with a null `last_task_at`, and the active repo sorts first.
    #[sqlx::test]
    async fn list_repositories_summarises_activity(pool: PgPool) {
        let active = upsert_repository(&pool, 1, "vymalo", "shop", "main")
            .await
            .unwrap();
        let idle = upsert_repository(&pool, 2, "vymalo", "idle", "trunk")
            .await
            .unwrap();

        for (n, delivery) in ["d-1", "d-2"].iter().enumerate() {
            // tasks.github_delivery_id FKs github_deliveries — record the delivery first, exactly as
            // the webhook handler does before creating a task.
            record_delivery(&pool, delivery, "pull_request", &json!({}))
                .await
                .unwrap();
            create_task(
                &pool,
                &NewTask {
                    repository_id: active,
                    installation_id: 7,
                    github_delivery_id: (*delivery).to_string(),
                    target_type: "pull_request".to_string(),
                    target_id: n as i64,
                    command_text: "review".to_string(),
                    base_sha: None,
                    head_sha: None,
                },
            )
            .await
            .unwrap();
        }

        let repos = list_repositories(&pool).await.unwrap();
        assert_eq!(repos.len(), 2);

        // Active repo (has tasks) sorts first by last_task_at.
        assert_eq!(repos[0].id, active);
        assert_eq!(repos[0].task_count, 2);
        assert!(repos[0].last_task_at.is_some());

        let idle_row = repos.iter().find(|r| r.id == idle).unwrap();
        assert_eq!(idle_row.task_count, 0);
        assert!(idle_row.last_task_at.is_none());
    }

    /// The runner's task context joins repository identity onto the task, and returns `None` for an
    /// unknown id (the seam the internal API serves to the agent runner).
    #[sqlx::test]
    async fn get_task_context_joins_repo_identity(pool: PgPool) {
        let repo_id = seed(&pool).await;
        let task_id = create_task(&pool, &pr_task(repo_id, "head1"))
            .await
            .unwrap()
            .unwrap();

        let context = get_task_context(&pool, task_id)
            .await
            .unwrap()
            .expect("task exists");
        assert_eq!(context.owner, "octo");
        assert_eq!(context.name, "repo");
        assert_eq!(context.default_branch, "main");
        assert_eq!(context.installation_id, 99);
        assert_eq!(context.command_text, "review");
        assert_eq!(context.head_sha.as_deref(), Some("head1"));

        assert!(
            get_task_context(&pool, Uuid::nil())
                .await
                .unwrap()
                .is_none(),
            "unknown id yields None"
        );
    }

    /// A terminal status stamps `completed_at` and clears the lease; `running` stamps `started_at`.
    /// `set_task_status` returns false for an unknown id (so the API can answer 404).
    #[sqlx::test]
    async fn set_task_status_stamps_and_releases(pool: PgPool) {
        let repo_id = seed(&pool).await;
        let task = claim_after_create(&pool, repo_id, "head1").await;

        assert!(set_task_status(&pool, task, "succeeded").await.unwrap());

        let row: (String, Option<OffsetDateTime>, Option<String>) =
            sqlx::query_as("SELECT status, completed_at, lease_owner FROM tasks WHERE id = $1")
                .bind(task)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.0, "succeeded");
        assert!(row.1.is_some(), "terminal status stamps completed_at");
        assert!(row.2.is_none(), "terminal status clears the lease");

        assert!(
            !set_task_status(&pool, Uuid::nil(), "failed").await.unwrap(),
            "unknown id reports no row updated"
        );
    }

    /// Create then claim a task (claim sets it `running` with a lease) so status-transition tests
    /// start from the state a dispatched task is really in.
    async fn claim_after_create(pool: &PgPool, repo_id: i64, head: &str) -> Uuid {
        create_task(pool, &pr_task(repo_id, head))
            .await
            .unwrap()
            .unwrap();
        claim_next_task(pool, "owner-a", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap()
            .id
    }
}
