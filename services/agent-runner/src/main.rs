//! Lightbridge agent runner.
//!
//! The per-task Kubernetes Job the dispatcher launches (ADR-0004). It holds no GitHub App key: it
//! reads its task id + control-plane callback wiring from env, fetches the task context (repo
//! coordinates + a short-lived installation token) from the control plane, clones the repo at the
//! relevant commit, runs the task, and reports a terminal status back. The control plane owns the
//! trust boundary — it mints the token and (in a later slice) validates findings and writes to
//! GitHub (ADR-0002, docs/opencode-acp-mcp.md).
//!
//! The lifecycle: clone → semantic index (tree-sitter → pgvector, slice 2) → structural index
//! (Graphify → Neo4j, slice 3) → review (OpenCode over the MCP tools, slice 5) → report. Indexing is
//! required; the structural graph and the review are best-effort and non-fatal.

use agent_runner::client::ControlPlaneClient;
use agent_runner::clone;
use agent_runner::config::{EmbeddingsConfig, ReviewConfig, RunnerConfig};
use agent_runner::embeddings::EmbeddingsClient;
use agent_runner::{indexer, review};

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

    // Optional JSON config file (ConfigMap-mounted); when absent, each config falls back to env. A
    // malformed file is a misconfiguration we surface as a failed task rather than silently ignore.
    let file_config = match agent_runner::config::load_file_config() {
        Ok(file_config) => file_config,
        Err(error) => {
            let detail = error.to_string();
            tracing::error!(%detail, "invalid agent config file");
            report(&client, &config, "failed", Some(&detail)).await;
            return std::process::ExitCode::FAILURE;
        }
    };

    let embeddings_config = match EmbeddingsConfig::resolve(file_config.as_ref()) {
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

    // Review is optional (no model → indexing-only). But if it's half-configured, surface it.
    let review_config = match ReviewConfig::resolve(file_config.as_ref()) {
        Ok(cfg) => cfg,
        Err(error) => {
            let detail = error.to_string();
            tracing::error!(%detail, "invalid review (LLM) configuration");
            report(&client, &config, "failed", Some(&detail)).await;
            return std::process::ExitCode::FAILURE;
        }
    };

    match run(&config, &client, &embedder, review_config.as_ref()).await {
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
    review_config: Option<&ReviewConfig>,
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

    // ── Semantic index: tree-sitter → pgvector (epic #5, slice 2) ────────────────────────────
    let chunk_count = indexer::index_checkout(&context, &checkout, client, embedder).await?;

    // ── Structural index: Graphify → Neo4j (epic #5, slice 3, ADR-0019) ──────────────────────
    // Best-effort: the semantic index already landed, and the graph store may be unconfigured
    // (control plane returns 503). A graph failure is logged, not fatal — the task still succeeds.
    let graph_summary = match indexer::graph::index_graph(&context, &checkout, client).await {
        Ok((nodes, edges)) => format!("{nodes} nodes / {edges} edges"),
        Err(error) => {
            tracing::warn!(%error, "structural graph indexing failed (non-fatal)");
            "graph skipped".to_string()
        }
    };

    // ── Review: OpenCode over the MCP tools → validated GitHub write-back ────────────────────
    // (slice 5 produces the review, slice 6 submits it; the control plane validates against the PR
    // diff and posts). Runs only when the LLM is configured; non-fatal (indexing already landed).
    let review_summary = match review_config {
        Some(review) => {
            // Scope the review to the PR's change set when we can compute it (best-effort; an
            // unavailable base commit just yields an unscoped review).
            let diff = clone::pr_diff(&checkout, &context).await;
            match review::run_review(&checkout, review, &context.command, diff.as_ref()).await {
                Ok(result) => {
                    tracing::info!(
                        findings = result.findings.len(),
                        summary = result.summary,
                        "review complete"
                    );
                    // Submit for validation + write-back (slice 6). Non-fatal: a post failure (e.g.
                    // GitHub hiccup) shouldn't fail a task whose indexing + review already succeeded.
                    match client.submit_review(config.task_id, &result).await {
                        Ok(()) => format!("{} findings posted", result.findings.len()),
                        Err(error) => {
                            tracing::warn!(%error, "submitting review failed (non-fatal)");
                            format!("{} findings (post failed)", result.findings.len())
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "review failed (non-fatal)");
                    "review failed".to_string()
                }
            }
        }
        None => "review disabled".to_string(),
    };

    Ok(format!(
        "indexed {}/{} at {} — {chunk_count} chunks, {graph_summary}; {review_summary}",
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
