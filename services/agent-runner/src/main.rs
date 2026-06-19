//! Lightbridge agent runner.
//!
//! The per-task Kubernetes Job the dispatcher launches (ADR-0004). It holds no GitHub App key: it
//! reads its task id + control-plane callback wiring from env, fetches the task context (repo
//! coordinates + a short-lived installation token) from the control plane, clones the repo at the
//! relevant commit, runs the task, and reports a terminal status back. The control plane owns the
//! trust boundary — it mints the token and (in a later slice) validates findings and writes to
//! GitHub (ADR-0002, docs/opencode-acp-mcp.md).
//!
//! This slice (epic #5, slice 1) wires the lifecycle end-to-end with a stubbed "work" step: it
//! proves clone + report so the indexer and OpenCode agent have a runner to live in.

use agent_runner::client::ControlPlaneClient;
use agent_runner::clone;
use agent_runner::config::RunnerConfig;

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

    match run(&config, &client).await {
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
async fn run(config: &RunnerConfig, client: &ControlPlaneClient) -> anyhow::Result<String> {
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

    // ── Stubbed work step (slice 1) ───────────────────────────────────────────────────────────
    // Indexing (tree-sitter → Neo4j + pgvector) and the OpenCode agent land in later slices of
    // epic #5. For now, prove the checkout is real and usable.
    let file_count = count_files(&checkout).await?;
    Ok(format!(
        "cloned {}/{} at {} ({file_count} files); indexing + agent not yet implemented",
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

/// Count tracked files under the checkout, skipping the `.git` directory. A cheap, real signal that
/// the clone produced a working tree — replaced by the indexer in slice 2.
async fn count_files(root: &std::path::Path) -> anyhow::Result<usize> {
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}
