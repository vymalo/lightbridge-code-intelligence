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

/// Reconcile the `code_chunks.embedding` column width to the configured `dimension` (ADR-0018). The
/// pgvector column is a fixed-width `vector(N)`, so changing the embedding model's dimension is
/// **destructive** — every stored vector is the wrong width. No-op when the column already matches
/// (or isn't present / has no fixed dim). On a mismatch: if `allow_clear`, **TRUNCATE `code_chunks`
/// and ALTER the column** to the new width; else return `Err` (fail loud) so a config typo can't
/// silently wipe the semantic index. Idempotent + safe to run from each role at startup.
pub async fn reconcile_embedding_dimension(
    pool: &PgPool,
    dimension: i64,
    allow_clear: bool,
) -> anyhow::Result<()> {
    use anyhow::bail;
    // pgvector stores the dimension in the column's `atttypmod` (== N for `vector(N)`, -1 if none).
    let current: Option<i32> = sqlx::query_scalar(
        "SELECT a.atttypmod FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         WHERE c.relname = 'code_chunks' AND a.attname = 'embedding' AND NOT a.attisdropped",
    )
    .fetch_optional(pool)
    .await?;
    let Some(current) = current.filter(|&m| m > 0).map(i64::from) else {
        return Ok(()); // no code_chunks/embedding column or no fixed dimension yet — nothing to do
    };
    if current == dimension {
        return Ok(());
    }
    if !allow_clear {
        bail!(
            "embedding dimension changed ({current} → {dimension}) but \
             embeddings.allow_reindex_on_dim_change is false; refusing to wipe code_chunks. \
             Set the flag to reindex from scratch, or revert the dimension."
        );
    }
    tracing::warn!(
        from = current,
        to = dimension,
        "embedding dimension changed; TRUNCATE code_chunks + ALTER column (reindex from scratch)"
    );
    sqlx::query("TRUNCATE TABLE code_chunks")
        .execute(pool)
        .await?;
    // `dimension` is an i64 from typed config (not user free-text), so formatting it into the DDL is
    // safe; the vector type width can't be a bind parameter.
    sqlx::query(&format!(
        "ALTER TABLE code_chunks ALTER COLUMN embedding TYPE vector({dimension})"
    ))
    .execute(pool)
    .await?;
    Ok(())
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
    /// Re-run discriminator (RFC-0001). `0` for the automatic first review; an explicit re-review
    /// (e.g. an `@mention`) uses the next epoch so the idempotency index lets a new task through for
    /// the same head SHA. See [`next_run_epoch`].
    pub run_epoch: i32,
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
         target_id, command_text, base_sha, head_sha, run_epoch, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'queued') \
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
    .bind(task.run_epoch)
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

/// Next `run_epoch` for an explicit re-run of a task's natural key: `max(run_epoch) + 1`, or `0` if
/// none exists yet. Lets a manual re-review (same head SHA) get past the idempotency index.
pub async fn next_run_epoch(
    pool: &PgPool,
    repository_id: i64,
    target_type: &str,
    target_id: i64,
    command_text: &str,
    head_sha: Option<&str>,
) -> Result<i32, sqlx::Error> {
    let next: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(run_epoch) + 1 FROM tasks \
         WHERE repository_id = $1 AND target_type = $2 AND target_id = $3 \
           AND command_text = $4 AND head_sha IS NOT DISTINCT FROM $5",
    )
    .bind(repository_id)
    .bind(target_type)
    .bind(target_id)
    .bind(command_text)
    .bind(head_sha)
    .fetch_one(pool)
    .await?;
    Ok(next.unwrap_or(0))
}

/// Cancel a PR's active tasks (queued/running/posting_result) — used when the PR is closed so its
/// work stops. Returns the cancelled task ids. The agent Jobs of cancelled tasks are deleted by the
/// reaper (the control plane that serves webhooks has no Kubernetes client — trust boundary).
pub async fn cancel_active_tasks_for_pr(
    pool: &PgPool,
    repository_id: i64,
    pr: i64,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE tasks SET status = 'cancelled', completed_at = now(), \
             lease_owner = NULL, lease_expires_at = NULL \
         WHERE repository_id = $1 AND target_type = 'pull_request' AND target_id = $2 \
           AND status IN ('queued', 'running', 'posting_result') \
         RETURNING id",
    )
    .bind(repository_id)
    .bind(pr)
    .fetch_all(pool)
    .await
}

/// Cancelled tasks that still have a Kubernetes Job to clean up (the reaper deletes the Job, then
/// clears `job_name` so the row isn't returned again).
pub async fn list_cancelled_with_job(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<ReapableTask>, sqlx::Error> {
    sqlx::query_as::<_, ReapableTask>(
        "SELECT id, job_name, attempts FROM tasks \
         WHERE status = 'cancelled' AND job_name IS NOT NULL \
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Clear a task's `job_name` once its Job has been deleted (so the cleanup is idempotent).
pub async fn clear_job_name(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET job_name = NULL WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
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

/// Return a `running` task to the queue with a backoff delay — used both when Job creation fails and
/// when the reaper requeues a stuck task. Clears the lease, `started_at`, and `job_name` so the next
/// claim is clean and the next dispatch creates a fresh Job (the Job name is derived from the task
/// id, so a stale name would otherwise collide). Guarded on the active statuses so two reapers can't
/// both requeue the same task. Returns `true` if a row was actually requeued.
pub async fn release_task(pool: &PgPool, id: Uuid, backoff: Duration) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE tasks \
         SET status = 'queued', lease_owner = NULL, lease_expires_at = NULL, started_at = NULL, \
             job_name = NULL, run_after = now() + ($2 * interval '1 second') \
         WHERE id = $1 AND status IN ('running', 'posting_result')",
    )
    .bind(id)
    .bind(backoff.as_secs_f64())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// A `running` task whose claim lease has expired — a candidate the reaper reconciles against its
/// Job's real liveness (RFC-0001 Phase 2).
#[derive(Debug, sqlx::FromRow)]
pub struct ReapableTask {
    pub id: Uuid,
    pub job_name: Option<String>,
    pub attempts: i32,
}

/// Tasks stuck in an active status (`running`/`posting_result`) past their lease — the lease is set
/// short at claim and renewed by the reaper only while the Job is live, so an expired lease just
/// means "needs reconciling", not "dead" — the caller decides by checking each Job's liveness.
/// Bounded so one cycle is cheap (backed by the `tasks_reapable_idx` partial index).
pub async fn list_reapable_tasks(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<ReapableTask>, sqlx::Error> {
    sqlx::query_as::<_, ReapableTask>(
        "SELECT id, job_name, attempts FROM tasks \
         WHERE status IN ('running', 'posting_result') AND lease_expires_at < now() \
         ORDER BY started_at NULLS FIRST \
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Extend an active task's lease — the reaper's heartbeat for a Job it confirmed is still live, so a
/// long-running task isn't reclaimed. No-op (returns `false`) if the task is no longer active.
pub async fn renew_lease(pool: &PgPool, id: Uuid, lease: Duration) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE tasks SET lease_expires_at = now() + ($2 * interval '1 second') \
         WHERE id = $1 AND status IN ('running', 'posting_result')",
    )
    .bind(id)
    .bind(lease.as_secs_f64())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
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

/// A semantic chunk submitted by the indexer runner (epic #5, slice 2).
pub struct CodeChunk {
    pub file_path: String,
    pub language: String,
    pub chunk_type: String,
    pub symbol_name: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
    pub content: String,
    /// 1536-dimensional embedding vector (text-embedding-3-small; see ADR-0018).
    pub embedding: Vec<f32>,
}

/// Upsert a batch of code chunks for a repository snapshot. The embedding is passed as a Postgres
/// vector literal so no extra crate is needed; `$N::vector` casts the text on the server side.
/// Runs in a single transaction; returns the number of rows inserted or updated.
pub async fn upsert_code_chunks(
    pool: &PgPool,
    repository_id: i64,
    commit_sha: &str,
    chunks: &[CodeChunk],
) -> anyhow::Result<usize> {
    use anyhow::Context;
    let mut tx = pool.begin().await.context("begin upsert transaction")?;
    let mut count = 0usize;
    for chunk in chunks {
        let emb = vector_literal(&chunk.embedding);
        sqlx::query(
            "INSERT INTO code_chunks \
             (repository_id, commit_sha, file_path, language, chunk_type, symbol_name, \
              start_line, end_line, content, embedding) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::vector) \
             ON CONFLICT (repository_id, commit_sha, file_path, start_line, end_line) \
             DO UPDATE SET \
               language    = EXCLUDED.language, \
               chunk_type  = EXCLUDED.chunk_type, \
               symbol_name = EXCLUDED.symbol_name, \
               content     = EXCLUDED.content, \
               embedding   = EXCLUDED.embedding",
        )
        .bind(repository_id)
        .bind(commit_sha)
        .bind(&chunk.file_path)
        .bind(&chunk.language)
        .bind(&chunk.chunk_type)
        .bind(&chunk.symbol_name)
        .bind(chunk.start_line)
        .bind(chunk.end_line)
        .bind(&chunk.content)
        .bind(&emb)
        .execute(&mut *tx)
        .await
        .context("upsert code_chunks row")?;
        count += 1;
    }
    tx.commit().await.context("commit upsert transaction")?;
    Ok(count)
}

/// Render a float slice as a pgvector text literal `[f0,f1,…]` in one pre-allocated buffer
/// (`$N::vector` casts it server-side, so no extra crate is needed).
fn vector_literal(v: &[f32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(v.len() * 12 + 2);
    s.push('[');
    for (i, f) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{f}");
    }
    s.push(']');
    s
}

/// One semantic-search hit (a `code_chunks` row + its similarity score). Serialized straight to the
/// retrieval API the vector MCP server calls.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CodeChunkHit {
    pub file_path: String,
    pub language: String,
    pub chunk_type: String,
    pub symbol_name: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
    pub content: String,
    /// Cosine similarity in `[0,1]` (`1 - cosine_distance`); higher is closer.
    pub score: f64,
}

/// Semantic search: the `limit` nearest chunks to `query_embedding` within one repo snapshot,
/// by cosine distance (the HNSW index's operator class). Scoped by `(repository_id, commit_sha)` so
/// a task only ever sees its own repo's index — the caller never picks the scope (trust boundary).
pub async fn search_code_chunks(
    pool: &PgPool,
    repository_id: i64,
    commit_sha: &str,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<CodeChunkHit>, sqlx::Error> {
    let emb = vector_literal(query_embedding);
    sqlx::query_as::<_, CodeChunkHit>(
        "SELECT file_path, language, chunk_type, symbol_name, start_line, end_line, content, \
                1.0 - (embedding <=> $1::vector) AS score \
         FROM code_chunks \
         WHERE repository_id = $2 AND commit_sha = $3 \
         ORDER BY embedding <=> $1::vector \
         LIMIT $4",
    )
    .bind(&emb)
    .bind(repository_id)
    .bind(commit_sha)
    .bind(limit)
    .fetch_all(pool)
    .await
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
            run_epoch: 0,
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

    /// Embedding-dimension reconcile: same dim → no-op; a change without the flag fails loud; with the
    /// flag it wipes + migrates the column to the new width.
    async fn embedding_dim(pool: &PgPool) -> i32 {
        sqlx::query_scalar::<_, i32>(
            "SELECT atttypmod FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
             WHERE c.relname = 'code_chunks' AND a.attname = 'embedding' AND NOT a.attisdropped",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn reconcile_embedding_dimension_guards_and_migrates(pool: PgPool) {
        // Migrations create code_chunks.embedding as vector(4096) (ADR-0018).
        assert_eq!(embedding_dim(&pool).await, 4096);

        // Same dimension → no-op.
        reconcile_embedding_dimension(&pool, 4096, false)
            .await
            .unwrap();
        assert_eq!(embedding_dim(&pool).await, 4096);

        // A change without the flag fails loud (no destruction).
        assert!(reconcile_embedding_dimension(&pool, 1536, false)
            .await
            .is_err());
        assert_eq!(
            embedding_dim(&pool).await,
            4096,
            "column untouched when not allowed"
        );

        // With the flag, the column migrates to the new width.
        reconcile_embedding_dimension(&pool, 1536, true)
            .await
            .unwrap();
        assert_eq!(embedding_dim(&pool).await, 1536);
    }

    /// `cancel_active_tasks_for_pr` cancels a PR's active task; `next_run_epoch` bumps so a manual
    /// re-review on the same head can create a new task (webhook re-trigger path).
    #[sqlx::test]
    async fn cancel_pr_and_next_run_epoch(pool: PgPool) {
        let repo_id = seed(&pool).await;
        let id = create_task(&pool, &pr_task(repo_id, "h1"))
            .await
            .unwrap()
            .unwrap();

        // One task exists at epoch 0 for this key → next is 1.
        let epoch = next_run_epoch(&pool, repo_id, "pull_request", 7, "review", Some("h1"))
            .await
            .unwrap();
        assert_eq!(epoch, 1);
        // A never-seen key starts at 0.
        let zero = next_run_epoch(&pool, repo_id, "pull_request", 999, "review", Some("x"))
            .await
            .unwrap();
        assert_eq!(zero, 0);

        // Closing the PR cancels its active task.
        let cancelled = cancel_active_tasks_for_pr(&pool, repo_id, 7).await.unwrap();
        assert_eq!(cancelled, vec![id]);
        let status: String = sqlx::query_scalar("SELECT status FROM tasks WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "cancelled");
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
                    run_epoch: 0,
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

    /// A 1536-dim one-hot vector (a 1.0 at `hot`, zeros elsewhere) — distinct directions give
    /// clean, predictable cosine ordering for the search test.
    fn one_hot(hot: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 1536];
        v[hot] = 1.0;
        v
    }

    fn chunk_at(file: &str, line: i32, hot: usize) -> CodeChunk {
        CodeChunk {
            file_path: file.to_string(),
            language: "rust".to_string(),
            chunk_type: "function".to_string(),
            symbol_name: Some(file.to_string()),
            start_line: line,
            end_line: line + 5,
            content: format!("// {file}"),
            embedding: one_hot(hot),
        }
    }

    /// Semantic search returns the nearest chunk first (cosine), scoped to the repo+commit, and
    /// honours the limit. Exercises the real pgvector `<=>` path + HNSW index.
    #[sqlx::test]
    async fn search_code_chunks_ranks_by_cosine_and_scopes(pool: PgPool) {
        let repo_id = seed(&pool).await;
        let chunks = vec![
            chunk_at("a.rs", 1, 0),
            chunk_at("b.rs", 1, 5),
            chunk_at("c.rs", 1, 9),
        ];
        upsert_code_chunks(&pool, repo_id, "headsha", &chunks)
            .await
            .unwrap();
        // A chunk on a *different* commit must not show up (scope check).
        upsert_code_chunks(&pool, repo_id, "othersha", &[chunk_at("a.rs", 1, 0)])
            .await
            .unwrap();

        // Query closest to the `hot=5` direction → b.rs ranks first with score ~1.0.
        let hits = search_code_chunks(&pool, repo_id, "headsha", &one_hot(5), 2)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2, "limit honoured");
        assert_eq!(hits[0].file_path, "b.rs");
        assert!(
            hits[0].score > 0.99,
            "exact direction ~1.0, got {}",
            hits[0].score
        );
        assert!(hits[0].score >= hits[1].score, "ordered by similarity");

        // Only this commit's chunks are searched (othersha excluded).
        let all = search_code_chunks(&pool, repo_id, "headsha", &one_hot(0), 50)
            .await
            .unwrap();
        assert_eq!(
            all.len(),
            3,
            "scoped to (repo, headsha) — othersha not included"
        );
    }
}
