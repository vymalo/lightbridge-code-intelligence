//! Lightbridge agent runner.
//!
//! The per-task Kubernetes Job the dispatcher launches (ADR-0004). It holds no GitHub App key: it
//! reads its task id + control-plane callback wiring from env, fetches the task context (repo
//! coordinates + a short-lived installation token) from the control plane, clones the repo at the
//! relevant commit, runs the task, and reports a terminal status back. The control plane owns the
//! trust boundary — it mints the token and (in a later slice) validates findings and writes to
//! GitHub (ADR-0002, docs/opencode-acp-mcp.md).
//!
//! Slice 1 wired the lifecycle end-to-end with a stubbed work step (clone + report). Slice 2 adds
//! the pgvector indexer: tree-sitter chunking + OpenAI-compatible embeddings → control-plane API.

use agent_runner::client::ControlPlaneClient;
use agent_runner::clone;
use agent_runner::config::{EmbeddingsConfig, RunnerConfig};
use agent_runner::embeddings::EmbeddingsClient;
use agent_runner::indexer;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let config = match RunnerConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            // No task id / callback wiring means we can't even report failure — just exit non-zero
            // so the Job is marked Failed and the dispatcher's reaper (Phase 2) can requeue it.
            tracing::error!(%error, "invalid runner configuration");
            return std::process::ExitCode::FAILURE;
        }
    };
    let client = ControlPlaneClient::new(&config.control_plane_url, &config.runner_token);

    let embeddings_config = match EmbeddingsConfig::from_env() {
        Ok(cfg) => cfg,
        Err(error) => {
            // The task is already `running` at this point; report failed so the dispatcher
            // doesn't wait for a lease timeout before it can reschedule.
            let detail = error.to_string();
            tracing::error!(%detail, "invalid embeddings configuration");
            report(&client, &config, "failed", Some(&detail)).await;
            return std::process::ExitCode::FAILURE;
        }
    };
    let embedder = EmbeddingsClient::new(
        &embeddings_config.base_url,
        &embeddings_config.api_key,
        &embeddings_config.model,
    );

    match run(&config, &client, &embedder).await {
        Ok(summary) => {
            tracing::info!(task_id = %config.task_id, summary, "task succeeded");
            report(&client, &config, "succeeded", None).await;
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            let detail = error.to_string();
            tracing::error!(task_id = %config.task_id, error = %detail, "task failed");
            report(&client, &config, "failed", Some(&detail)).await;
            std::process::ExitCode::FAILURE
        }
    }
}

/// The task lifecycle. Returns a human summary on success; any error is reported as `failed`.
async fn run(
    config: &RunnerConfig,
    client: &ControlPlaneClient,
    embedder: &EmbeddingsClient,
) -> anyhow::Result<String> {
    // Mark that the runner actually started (the dispatcher already set `running` on claim; this
    // re-affirms it from the pod and is a no-op if already set).
    report(client, config, "running", None).await;

    let context = client.get_context(config.task_id).await?;
    tracing::info!(
        repo = format!("{}/{}", context.owner, context.name),
        command = context.command,
        target = format!("{}#{}", context.target_type, context.target_id),
        head_sha = context.head_sha.as_deref().unwrap_or("(default branch)"),
        "fetched task context"
    );

    let checkout = clone::checkout(&context, &config.workdir).await?;

    // ── Indexing: tree-sitter → pgvector (epic #5, slice 2) ──────────────────────────────────
    let chunk_count = indexer::index_checkout(&context, &checkout, client, embedder).await?;

    Ok(format!(
        "indexed {}/{} at {} — {chunk_count} chunks submitted",
        context.owner,
        context.name,
        context
            .head_sha
            .as_deref()
            .unwrap_or(&context.default_branch),
    ))
}

/// Best-effort status report: a failed report must not mask the task's real outcome, so we log and
/// move on rather than propagate (the lease/reaper recovers a task whose final report was lost).
async fn report(
    client: &ControlPlaneClient,
    config: &RunnerConfig,
    status: &str,
    detail: Option<&str>,
) {
    if let Err(error) = client.report_status(config.task_id, status, detail).await {
        tracing::warn!(%error, task_id = %config.task_id, status, "failed to report status");
    }
}
