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

/// The default reviewer **guidance** — persona + what to optimise for. Operators can replace this
/// via `REVIEW_SYSTEM_PROMPT` (see [`ReviewConfig`]); the fixed [`OUTPUT_CONTRACT`] is always appended
/// afterwards, so an override changes *behaviour* without ever breaking the machine-readable result.
///
/// It is tuned for the human on the other end: high-signal, terse, skimmable. The goal is the most
/// useful review a person can act on in seconds — not the longest one.
pub const DEFAULT_REVIEW_GUIDANCE: &str = "\
You are Lightbridge, an expert pull-request reviewer. Review ONLY the change in the unified diff \
below; use the rest of the repository and the tools as CONTEXT to judge its impact, never as review \
targets.\n\n\
Optimise for the reviewer's time — a human should grasp each finding in seconds:\n\
- Report only what matters: correctness bugs, security issues, data loss, and mismatches with the \
change's stated intent. Skip style/nits unless they cause real harm.\n\
- Be brief and concrete. Title ≤ ~8 words. Body 1–2 sentences: what's wrong and why it matters — no \
restating the code, no hedging, no praise.\n\
- Favour a few high-confidence findings over many shallow ones. If the change is sound, say so in \
one line and return no findings — silence is better than noise.\n\
- Ground every claim with the tools; do not speculate about code you have not looked up:\n\
  - `lightbridge_vector_semantic_search` — find related code by meaning,\n\
  - `lightbridge_graph_find_symbol` / `lightbridge_graph_get_callers` — trace structure and impact.\n\
- Reason old-vs-new per hunk. When a fix is clear, give the exact replacement so the author applies \
it in one click. You may not edit files or run commands.\n\n\
Keep `summary` to 1–3 sentences: does the change do what it intends, and is it correct and safe?";

/// The fixed output contract appended after the guidance and the diff. The parser ([`parse_review`])
/// and the control plane's scope-and-suggestion handling depend on this exact shape, so it is NOT
/// operator-overridable.
pub const OUTPUT_CONTRACT: &str = "\
Scope rule (non-negotiable): every finding's `line` MUST be a line this diff adds or changes; never \
comment on untouched code. Set `severity` to `error` (must fix), `warning` (should fix), or `info` \
(minor/FYI). When you propose a fix, put the EXACT replacement source for that one line in \
`suggestion` (no diff markers, no fences) so it applies as a GitHub suggestion.\n\n\
Output ONLY a single fenced ```json block with this exact shape and nothing after it:\n\
{\n  \"summary\": \"1–3 sentences\",\n  \"findings\": [\n    {\"file\": \"path/from/repo/root\", \"line\": 42, \"severity\": \"info|warning|error\", \"title\": \"short\", \"body\": \"why it matters\", \"suggestion\": \"optional exact replacement for line 42\"}\n  ]\n}";

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
    let guidance = review
        .system_prompt
        .as_deref()
        .unwrap_or(DEFAULT_REVIEW_GUIDANCE);
    let prompt = build_prompt(guidance, command, diff);

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
fn build_prompt(guidance: &str, command: &str, diff: Option<&PrDiff>) -> String {
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
    fn default_prompt_carries_guidance_and_the_fixed_contract() {
        let prompt = build_prompt(DEFAULT_REVIEW_GUIDANCE, "review", None);
        assert!(
            prompt.contains("expert pull-request reviewer"),
            "uses default guidance"
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
        let prompt = build_prompt("CUSTOM PERSONA", "review", Some(&diff));
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
