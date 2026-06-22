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
pub mod native;
mod parse;

pub use config::opencode_config;
pub use parse::{parse_review, ReviewFinding, ReviewResult};

use std::path::Path;

use anyhow::Context;

use crate::bootstrap::config::ReviewConfig;
use crate::clone::PrDiff;

/// The provider id we register the eaig gateway under in opencode.json; the model is referenced as
/// `eaig/<model>`.
pub const PROVIDER_ID: &str = "eaig";

/// The native agent's entry point (ADR-0037): the unified loop that acts via mediated write tools.
/// Re-exported here so callers use `review::run_native_agent`.
pub use native::agent::run_native_agent;

/// The fixed output contract for the **OpenCode** path (the JSON-block shape its parser
/// [`parse_review`] depends on). The native agent does not use this — it acts via tools (ADR-0037);
/// this constant retires with OpenCode (#140). It is a machine contract, not operator-tunable.
pub const OUTPUT_CONTRACT: &str = "\
Scope rule (non-negotiable): every finding's `line` MUST be a line this diff adds or changes; never \
comment on untouched code. Give each finding a `priority` — `P0` (must fix), `P1` (should fix), or \
`P2` (minor/FYI) — and a `category` — `security`, `correctness`, `quality`, `style`, or `performance`. \
Each finding also has a short `title`, a `body` that is the detailed explanation, an optional \
`suggestion`, and optional `resources`. When you propose a fix, put the EXACT replacement source for \
that one line in `suggestion` (no diff markers, no fences) so it applies as a GitHub suggestion. Put \
any supporting links (docs, CWE, RFCs) in `resources` as full URLs.\n\n\
Output ONLY a single fenced ```json block with this exact shape and nothing after it:\n\
{\n  \"summary\": \"1–3 sentences\",\n  \"findings\": [\n    {\"file\": \"path/from/repo/root\", \"line\": 42, \"priority\": \"P0\", \"category\": \"security\", \"title\": \"short\", \"body\": \"detailed explanation of why it matters\", \"suggestion\": \"optional exact replacement for line 42\", \"resources\": [\"https://...\"]}\n  ]\n}";

/// Run the OpenCode review over `checkout` and return the parsed structured result (the
/// `REVIEW_AGENT=opencode` fallback). The native agent (default) acts via [`run_native_agent`]
/// instead. OpenCode retires in #140.
///
/// Writes `opencode.json` into `checkout` (ephemeral Job disk), then spawns
/// `opencode run --model eaig/<model> --dir <checkout> --print-logs <prompt>` and parses its output.
pub async fn run_opencode(
    checkout: &Path,
    review: &ReviewConfig,
    command: &str,
    diff: Option<&PrDiff>,
    attribution: &[(String, String)],
) -> anyhow::Result<ReviewResult> {
    // Generate the project config the agent runs under. Built from the runner's own env so the MCP
    // subprocesses inherit TASK_ID / CONTROL_PLANE_URL / AGENT_RUNNER_TOKEN / EMBEDDINGS_* explicitly
    // rather than relying on env inheritance. `attribution` adds the gateway billing headers (#89).
    let cfg = opencode_config(review, &mcp_env(), attribution);
    let cfg_path = checkout.join("opencode.json");
    tokio::fs::write(&cfg_path, serde_json::to_vec_pretty(&cfg)?)
        .await
        .with_context(|| format!("writing {}", cfg_path.display()))?;

    let model = format!("{PROVIDER_ID}/{}", review.model);
    // The operator-owned guidance (required, ADR-0037 — no built-in default); the fixed
    // OUTPUT_CONTRACT is appended last by `build_prompt`.
    let prompt = build_prompt(&review.system_prompt, command, diff, review.max_diff_chars);

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

/// Assemble the full agent prompt: the (configurable) `guidance`, the requested command, the changed
/// files + unified diff when we have them, and the fixed [`OUTPUT_CONTRACT`] last. Without a diff
/// (a non-PR run, or the base commit wasn't available) we fall back to an unscoped review over the
/// checkout, still steered by the command. The contract goes last so it's the final instruction the
/// model sees regardless of how long the guidance or diff are.
fn build_prompt(
    guidance: &str,
    command: &str,
    diff: Option<&PrDiff>,
    max_diff_chars: usize,
) -> String {
    let mut prompt = format!("{guidance}\n\nRequested review command: {command}");
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
            if pr.diff.len() > max_diff_chars {
                // Truncate on a char boundary so we never slice through a multi-byte sequence.
                let mut end = max_diff_chars;
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
    // The fixed contract is always last so it's the final instruction, even after a long custom
    // guidance or a large diff.
    prompt.push_str("\n\n");
    prompt.push_str(OUTPUT_CONTRACT);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_prompt_carries_guidance_and_the_fixed_contract() {
        // OpenCode path: the operator guidance (required, no default) + the fixed JSON contract last.
        let prompt = build_prompt("CUSTOM REVIEWER PERSONA", "review", None, 60_000);
        assert!(
            prompt.contains("CUSTOM REVIEWER PERSONA"),
            "uses the supplied guidance"
        );
        assert!(prompt.contains("Requested review command: review"));
        assert!(
            prompt.contains("No diff is available"),
            "unscoped fallback note"
        );
        // The machine contract is always present and last.
        assert!(prompt.contains("Scope rule (non-negotiable)"));
        assert!(
            prompt.trim_end().ends_with("]\n}"),
            "ends with the JSON shape"
        );
    }

    #[test]
    fn custom_guidance_overrides_but_contract_and_diff_remain() {
        let diff = PrDiff {
            diff: "@@ -1 +1 @@\n-old\n+new".to_string(),
            files: vec!["src/x.rs".to_string()],
        };
        let prompt = build_prompt("CUSTOM PERSONA", "review", Some(&diff), 60_000);
        assert!(prompt.contains("CUSTOM PERSONA"), "operator guidance used");
        assert!(
            !prompt.contains("expert pull-request reviewer"),
            "default not appended"
        );
        assert!(prompt.contains("This PR changes 1 file(s)") && prompt.contains("src/x.rs"));
        assert!(prompt.contains("+new"), "diff is included");
        // The contract still wins the last word, so parsing stays intact under any override.
        let contract_at = prompt
            .find("Scope rule (non-negotiable)")
            .expect("contract present");
        let diff_at = prompt.find("+new").expect("diff present");
        assert!(contract_at > diff_at, "contract comes after the diff");
    }
}
