//! Code review via OpenCode (epic #5, slice 5; ADR-0021).
//!
//! The runner spawns OpenCode headless (`opencode run`) over the checkout, with a generated
//! `opencode.json` that wires (a) the eaig OpenAI-compatible LLM provider and (b) our two stdio MCP
//! servers (vector + graph) so the agent can investigate the repo. OpenCode reasons and emits a
//! structured review payload, which the runner parses. Validation + GitHub write-back is slice 6 —
//! this slice stops at *producing* the structured result.
//!
//! Why `opencode run` and not ACP: `run` is OpenCode's documented headless/scripting entrypoint;
//! ACP is its editor-integration protocol. Verified locally that OpenCode connects to our MCP
//! servers from a project `opencode.json` (`opencode mcp list` → `✓ connected`).

mod config;
mod parse;

pub use config::opencode_config;
pub use parse::{parse_review, ReviewFinding, ReviewResult};

use std::path::Path;

use anyhow::Context;

use crate::config::ReviewConfig;

/// The provider id we register the eaig gateway under in opencode.json; the model is referenced as
/// `eaig/<model>`.
pub const PROVIDER_ID: &str = "eaig";

/// The system instruction handed to the review agent. It must end by emitting the structured result
/// (parsed by [`parse_review`]) so the runner gets machine-readable findings, not prose.
pub const REVIEW_PROMPT: &str = "\
You are Lightbridge, a precise code reviewer. Review the repository at the current working \
directory for the requested change. Use the available tools to ground every claim:\n\
- `lightbridge_vector_semantic_search` to find related code by meaning,\n\
- `lightbridge_graph_find_symbol` and `lightbridge_graph_get_callers` to trace structure and impact.\n\
Do not speculate about code you have not looked up. Prefer a few high-confidence findings over many \
shallow ones. You may not edit files or run commands.\n\n\
When done, output ONLY a single fenced ```json block with this exact shape and nothing after it:\n\
{\n  \"summary\": \"one-paragraph overall assessment\",\n  \"findings\": [\n    {\"file\": \"path/from/repo/root\", \"line\": 42, \"severity\": \"info|warning|error\", \"title\": \"short\", \"body\": \"explanation grounded in the tools\"}\n  ]\n}";

/// Run the OpenCode review over `checkout` and return the parsed structured result.
///
/// Writes `opencode.json` into `checkout` (ephemeral Job disk), then spawns
/// `opencode run --model eaig/<model> --dir <checkout> --print-logs <prompt>` and parses its output.
pub async fn run_review(
    checkout: &Path,
    review: &ReviewConfig,
    command: &str,
) -> anyhow::Result<ReviewResult> {
    // Generate the project config the agent runs under. Built from the runner's own env so the MCP
    // subprocesses inherit TASK_ID / CONTROL_PLANE_URL / AGENT_RUNNER_TOKEN / EMBEDDINGS_* explicitly
    // rather than relying on env inheritance.
    let cfg = opencode_config(review, &mcp_env());
    let cfg_path = checkout.join("opencode.json");
    tokio::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)
        .await
        .with_context(|| format!("writing {}", cfg_path.display()))?;

    let model = format!("{PROVIDER_ID}/{}", review.model);
    let prompt = format!("{REVIEW_PROMPT}\n\nRequested review command: {command}");

    let output = tokio::process::Command::new("opencode")
        .arg("run")
        .arg("--model")
        .arg(&model)
        .arg("--dir")
        .arg(checkout)
        .arg("--print-logs")
        .arg(&prompt)
        .output()
        .await
        .context("spawning opencode (is it on PATH in the image?)")?;

    if !output.status.success() {
        anyhow::bail!(
            "opencode run exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_review(&stdout).context("parsing the review result from opencode output")
}

/// The env the MCP subprocesses need, read from the runner's own process env (so secrets aren't
/// hard-coded). Only the names the MCP servers require; missing ones are simply omitted.
fn mcp_env() -> Vec<(String, String)> {
    [
        "TASK_ID",
        "CONTROL_PLANE_URL",
        "AGENT_RUNNER_TOKEN",
        "EMBEDDINGS_BASE_URL",
        "EMBEDDINGS_API_KEY",
        "EMBEDDINGS_MODEL",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
    .collect()
}
