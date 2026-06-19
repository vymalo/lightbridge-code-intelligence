//! Dispatcher role (RFC-0001 Phase 1): claim queued tasks and launch one Kubernetes Job per task.
//!
//! The loop drains all currently-due tasks, then blocks until a `LISTEN/NOTIFY` wakeup or a short
//! poll fallback — the poll covers a missed notification so work is never stranded. Claiming uses
//! `SELECT … FOR UPDATE SKIP LOCKED`, so any number of dispatcher replicas can run concurrently
//! without ever claiming the same task.

use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;

use crate::db;
use crate::k8s::TaskLauncher;

/// How long a claim lease lasts before the scheduler's reaper may reclaim it (Phase 2). Kept short:
/// it only needs to cover Job creation, not the Job's whole runtime.
const CLAIM_LEASE: Duration = Duration::from_secs(60);
/// Backoff before a task whose Job failed to launch is retried.
const LAUNCH_BACKOFF: Duration = Duration::from_secs(30);
/// Fallback poll cadence in case a `NOTIFY` is missed (e.g. enqueued while we were busy).
const POLL_FALLBACK: Duration = Duration::from_secs(5);

/// Run the dispatcher until cancelled. `owner` identifies this replica in the lease (e.g. the pod
/// name) for observability and Phase-2 reaping.
pub async fn run<L: TaskLauncher>(pool: PgPool, launcher: L, owner: String) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(db::TASK_QUEUED_CHANNEL).await?;
    tracing::info!(owner, "dispatcher started");

    loop {
        drain(&pool, &launcher, &owner).await;

        // Wait for an enqueue notification or the poll fallback, whichever comes first.
        tokio::select! {
            recv = listener.recv() => {
                if let Err(error) = recv {
                    // The listener connection dropped; log and let the poll cadence drive recovery.
                    tracing::warn!(%error, "notify listener error; falling back to polling");
                    tokio::time::sleep(POLL_FALLBACK).await;
                }
            }
            _ = tokio::time::sleep(POLL_FALLBACK) => {}
        }
    }
}

/// Claim and dispatch every task that is due right now, then return so the caller can wait.
async fn drain<L: TaskLauncher>(pool: &PgPool, launcher: &L, owner: &str) {
    loop {
        match db::claim_next_task(pool, owner, CLAIM_LEASE).await {
            Ok(Some(task)) => dispatch(pool, launcher, &task).await,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(%error, "failed to claim next task");
                return;
            }
        }
    }
}

/// Launch a claimed task's Job and record it; on failure, requeue with backoff so the work is not
/// lost (the claim already moved it out of `queued`).
async fn dispatch<L: TaskLauncher>(pool: &PgPool, launcher: &L, task: &db::ClaimedTask) {
    let started = std::time::Instant::now();
    match launcher.launch(task).await {
        Ok(job_name) => {
            crate::metrics::dispatch_outcome("launched");
            crate::metrics::dispatch_claim_seconds(started.elapsed().as_secs_f64());
            match db::set_task_job(pool, task.id, &job_name).await {
                Ok(()) => tracing::info!(task_id = %task.id, job_name, "dispatched task to a Job"),
                Err(error) => {
                    // The Job exists but we couldn't record its name; surface it for follow-up
                    // rather than launching a second Job.
                    tracing::error!(%error, task_id = %task.id, job_name, "failed to record job name")
                }
            }
        }
        Err(error) => {
            crate::metrics::dispatch_outcome("failed");
            tracing::error!(%error, task_id = %task.id, "failed to launch job; requeueing");
            if let Err(release_error) = db::release_task(pool, task.id, LAUNCH_BACKOFF).await {
                tracing::error!(%release_error, task_id = %task.id, "failed to requeue task");
            }
        }
    }
}
