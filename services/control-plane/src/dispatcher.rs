//! Dispatcher role (RFC-0001 Phase 1 + Phase 2 reaper): claim queued tasks, launch one Kubernetes
//! Job per task, and reconcile stuck tasks.
//!
//! The loop drains all currently-due tasks, then blocks until a `LISTEN/NOTIFY` wakeup, the reap
//! tick, or a short poll fallback — the poll covers a missed notification so work is never stranded.
//! Claiming uses `SELECT … FOR UPDATE SKIP LOCKED`, so any number of dispatcher replicas can run
//! concurrently without ever claiming the same task. Loop timings come from the file config (else
//! defaults). The reaper shares this loop (singleton today; idempotent writes keep it correct on N).

use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;

use crate::db;
use crate::k8s::TaskLauncher;
use crate::reaper;

/// Defaults for the dispatcher timings when the file config doesn't set them.
const DEFAULT_CLAIM_LEASE_SECS: u64 = 60;
const DEFAULT_POLL_FALLBACK_SECS: u64 = 5;
const DEFAULT_LAUNCH_BACKOFF_SECS: u64 = 30;
const DEFAULT_REAP_INTERVAL_SECS: u64 = 30;

/// Tunable dispatcher loop timings.
#[derive(Debug, Clone, Copy)]
pub struct DispatcherConfig {
    /// Claim lease before the reaper may reconcile a task (Phase 2). Kept short: it only covers Job
    /// creation; the reaper renews it while the Job is live.
    pub claim_lease: Duration,
    /// Fallback poll cadence in case a `NOTIFY` is missed (e.g. enqueued while we were busy).
    pub poll_fallback: Duration,
    /// Backoff before a task whose Job failed to launch is retried.
    pub launch_backoff: Duration,
    /// How often the reaper reconciles stuck (lease-expired) tasks against their Jobs.
    pub reap_interval: Duration,
}

impl DispatcherConfig {
    /// Resolve from the file config's `dispatcher` section; each unset (or zero) field uses its
    /// default.
    pub fn from_file(section: Option<&crate::config::DispatcherSection>) -> Self {
        let secs = |value: Option<u64>, default: u64| {
            Duration::from_secs(value.filter(|&s| s > 0).unwrap_or(default))
        };
        Self {
            claim_lease: secs(
                section.and_then(|s| s.claim_lease_seconds),
                DEFAULT_CLAIM_LEASE_SECS,
            ),
            poll_fallback: secs(
                section.and_then(|s| s.poll_fallback_seconds),
                DEFAULT_POLL_FALLBACK_SECS,
            ),
            launch_backoff: secs(
                section.and_then(|s| s.launch_backoff_seconds),
                DEFAULT_LAUNCH_BACKOFF_SECS,
            ),
            reap_interval: secs(
                section.and_then(|s| s.reap_interval_seconds),
                DEFAULT_REAP_INTERVAL_SECS,
            ),
        }
    }
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self::from_file(None)
    }
}

/// Run the dispatcher until cancelled. `owner` identifies this replica in the lease (e.g. the pod
/// name) for observability and Phase-2 reaping.
pub async fn run<L: TaskLauncher>(
    pool: PgPool,
    launcher: L,
    owner: String,
    cfg: DispatcherConfig,
    neo4j: Option<std::sync::Arc<neo4rs::Graph>>,
) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(db::TASK_QUEUED_CHANNEL).await?;
    // The reaper shares this loop (the dispatcher is a singleton today); its writes are idempotent
    // and active-status-guarded, so it stays correct even if more than one replica runs it.
    let mut reap_tick = tokio::time::interval(cfg.reap_interval);
    reap_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(owner, "dispatcher started");

    loop {
        drain(&pool, &launcher, &owner, &cfg).await;

        // Wait for an enqueue notification, the reap tick, the poll fallback, or shutdown.
        tokio::select! {
            recv = listener.recv() => {
                if let Err(error) = recv {
                    // The listener connection dropped; log and let the poll cadence drive recovery.
                    tracing::warn!(%error, "notify listener error; falling back to polling");
                    tokio::time::sleep(cfg.poll_fallback).await;
                }
            }
            _ = reap_tick.tick() => {
                if let Err(error) = reaper::reap_once(&pool, &launcher).await {
                    tracing::error!(%error, "reaper cycle failed");
                }
                // Durable backstop for repo data purge (a spawned purge can be lost on restart).
                crate::lifecycle::reconcile_purges(&pool, neo4j.as_deref()).await;
            }
            _ = tokio::time::sleep(cfg.poll_fallback) => {}
            // Graceful shutdown (e.g. a deploy SIGTERMs the pod): stop the loop between iterations so
            // we never die mid-claim/launch leaving a task claimed-but-not-launched. In-flight Jobs
            // keep running independently; the successor's reaper reconciles them.
            _ = shutdown_signal() => {
                tracing::info!(owner, "received shutdown signal; stopping dispatcher loop");
                break;
            }
        }
    }
    Ok(())
}

/// Resolves on SIGTERM (Kubernetes pod termination) or Ctrl-C, for a clean dispatcher shutdown. We
/// run on Linux/macOS; the non-Unix arm falls back to Ctrl-C so the code still compiles.
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        // Can't install the handler — never resolve (the orchestrator's SIGKILL still stops us).
        Err(error) => {
            tracing::warn!(%error, "could not install SIGTERM handler");
            return std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "could not install Ctrl-C handler");
        std::future::pending::<()>().await;
    }
}

/// Claim and dispatch every task that is due right now, then return so the caller can wait.
async fn drain<L: TaskLauncher>(pool: &PgPool, launcher: &L, owner: &str, cfg: &DispatcherConfig) {
    loop {
        match db::claim_next_task(pool, owner, cfg.claim_lease).await {
            Ok(Some(task)) => dispatch(pool, launcher, &task, cfg).await,
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
async fn dispatch<L: TaskLauncher>(
    pool: &PgPool,
    launcher: &L,
    task: &db::ClaimedTask,
    cfg: &DispatcherConfig,
) {
    let started = std::time::Instant::now();
    match launcher.launch(task).await {
        Ok(job_name) => {
            crate::metrics::dispatch_outcome("launched");
            crate::metrics::dispatch_launch_seconds(started.elapsed().as_secs_f64());
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
            if let Err(release_error) = db::release_task(pool, task.id, cfg.launch_backoff).await {
                tracing::error!(%release_error, task_id = %task.id, "failed to requeue task");
            }
        }
    }
}
