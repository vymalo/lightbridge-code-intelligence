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

use crate::clone::PrDiff;
use crate::config::ReviewConfig;

/// The provider id we register the eaig gateway under in opencode.json; the model is referenced as
/// `eaig/<model>`.
pub const PROVIDER_ID: &str = "eaig";

/// Upper bound on the diff we paste into the prompt; beyond this we truncate and tell the model so it
/// reviews what it can rather than blowing the context window on a huge PR.
const MAX_DIFF_CHARS: usize = 60_000;

/// The system instruction handed to the review agent. It must end by emitting the structured result
/// (parsed by [`parse_review`]) so the runner gets machine-readable findings, not prose.
///
/// Per the AI-governance working agreement, this is a *pull-request* review: scoped to the change,
/// grounded in evidence, severity-tagged, and offering concrete fixes — not a whole-repo audit.
pub const REVIEW_PROMPT: &str = "\
You are Lightbridge, a precise pull-request reviewer. You review the PULL REQUEST described by the \
unified diff below — NOT the whole repository.\n\n\
Rules:\n\
- **Scope:** every finding MUST land on a line the diff adds or changes (a `+` line in a hunk). Use \
the rest of the repository and the tools only as CONTEXT to judge the change's impact — never raise \
a finding about code this PR does not touch.\n\
- **Ground every claim** with the tools; do not speculate about code you have not looked up:\n\
  - `lightbridge_vector_semantic_search` — find related code by meaning,\n\
  - `lightbridge_graph_find_symbol` / `lightbridge_graph_get_callers` — trace structure and impact.\n\
- **Compare old vs new:** reason about what each hunk changed and what it breaks or improves.\n\
- Prefer a few high-confidence findings over many shallow ones. You may not edit files or run \
commands.\n\n\
Shape your `summary` around: (1) does the change match its stated intent/scope, (2) correctness, \
(3) security, (4) tests/verification. For each finding set `severity` to `error` (must fix), \
`warning` (should fix), or `info` (note). When you can propose a concrete fix, include a \
`suggestion` field containing the EXACT replacement source for that one line (no diff markers, no \
fences) so it can be applied as a GitHub suggestion.\n\n\
When done, output ONLY a single fenced ```json block with this exact shape and nothing after it:\n\
{\n  \"summary\": \"assessment covering intent/scope, correctness, security, tests\",\n  \"findings\": [\n    {\"file\": \"path/from/repo/root\", \"line\": 42, \"severity\": \"info|warning|error\", \"title\": \"short\", \"body\": \"explanation grounded in the tools\", \"suggestion\": \"optional exact replacement for line 42\"}\n  ]\n}";

/// Run the OpenCode review over `checkout` and return the parsed structured result.
///
/// Writes `opencode.json` into `checkout` (ephemeral Job disk), then spawns
/// `opencode run --model eaig/<model> --dir <checkout> --print-logs <prompt>` and parses its output.
pub async fn run_review(
    checkout: &Path,
    review: &ReviewConfig,
    command: &str,
    diff: Option<&PrDiff>,
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
    let prompt = build_prompt(command, diff);

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

/// Assemble the full agent prompt: the system instruction, the requested command, and — when we have
/// it — the changed-file list plus the unified diff that scopes the review. Without a diff (e.g. a
/// non-PR run, or the base commit wasn't available) we fall back to an unscoped review over the
/// checkout, still steered by the command.
fn build_prompt(command: &str, diff: Option<&PrDiff>) -> String {
    let mut prompt = format!("{REVIEW_PROMPT}\n\nRequested review command: {command}");
    match diff {
        Some(pr) => {
            prompt.push_str(&format!(
                "\n\nThis PR changes {} file(s):\n{}",
                pr.files.len(),
                pr.files
                    .iter()
                    .map(|f| format!("- {f}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
            prompt.push_str("\n\nUnified diff (review ONLY lines this diff changes):\n```diff\n");
            if pr.diff.len() > MAX_DIFF_CHARS {
                // Truncate on a char boundary so we never slice through a multi-byte sequence.
                let mut end = MAX_DIFF_CHARS;
                while !pr.diff.is_char_boundary(end) {
                    end -= 1;
                }
                prompt.push_str(&pr.diff[..end]);
                prompt.push_str("\n… [diff truncated; review the hunks shown above] …");
            } else {
                prompt.push_str(&pr.diff);
            }
            prompt.push_str("\n```");
        }
        None => prompt.push_str(
            "\n\nNo diff is available for this run; review the working tree for the requested \
             change and keep findings grounded in the tools.",
        ),
    }
    prompt
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
