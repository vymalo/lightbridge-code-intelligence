# ADR-0021 — OpenCode (headless `run`) as the review agent

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-19 |
| Deciders   | @ssegning |
| Epic       | #5 (indexer + agent, slice 5) |
| Builds on  | [ADR-0018](0018-openai-compatible-embeddings.md), [ADR-0020](0020-mcp-servers-via-control-plane.md) |

## Context

Slice 5 makes the agent *reason*: investigate the repo via the MCP tools (slice 4) and produce a
structured review. `docs/opencode-acp-mcp.md` envisioned the runner driving OpenCode over **ACP**
(the Rust runner as ACP client). On inspecting the actual OpenCode (1.17.3):

- `opencode run [message]` is the documented **headless/scripting** entrypoint
  (`--model provider/model`, `--agent`, `--format default|json`, `--dir`). ACP (`opencode acp`) is
  its **editor-integration** protocol — heavier and aimed at IDEs, not a batch Job.
- OpenCode reads a project `opencode.json` for providers + MCP servers. Verified locally that it
  spawns our compiled MCP servers and reports `✓ connected` (`opencode mcp list`), and that it
  accepts our generated config (eaig provider + both MCP servers + read-only permissions).

## Decision

**Drive OpenCode headlessly with `opencode run`, configured by a generated `opencode.json`.** Not
ACP — `run` is the right batch entrypoint, and we avoid implementing an ACP client.

- The runner writes `opencode.json` into the checkout wiring:
  - the **eaig** OpenAI-compatible LLM provider (`@ai-sdk/openai-compatible`, `baseURL` from
    `LLM_BASE_URL`, key via `{env:LLM_API_KEY}`); model referenced as `eaig/<LLM_MODEL>`. Same
    gateway as embeddings (ADR-0018); **no default model** — review is skipped unless `LLM_MODEL` is
    set.
  - our two stdio MCP servers (`lightbridge_vector`, `lightbridge_graph`), env injected from the
    runner's process env.
  - `permission: { edit: deny, bash: deny, webfetch: deny }` — the reviewer reads only via the MCP
    tools; the untrusted repo can't trick it into running code.
- The runner spawns `opencode run --model eaig/<model> --dir <checkout> <prompt>` and parses the
  agent's final fenced ```json block into a `ReviewResult` (`summary` + `findings[]`).
- The review step is **optional + non-fatal**: indexing has already succeeded, and validation +
  GitHub write-back is slice 6. A review failure (or no LLM configured) is logged, not fatal.

OpenCode is bundled into the existing combined image via its official installer (a standalone
binary; no Node runtime).

## Consequences

**Good**
- Reuses OpenCode's agent loop, tool-calling, and provider plumbing instead of building one.
- Read-only, MCP-only repo access keeps the untrusted-repo blast radius small.
- Structured `ReviewResult` gives slice 6 a clean contract to validate + post.

**Trade-offs**
- The image grows again (OpenCode binary). Accepted — one image with everything.
- We parse a fenced JSON block from the model's output; prompt-dependent. Guarded by a parser unit
  test, and slice 6 re-validates every finding's line ref before posting.
- At runtime OpenCode may fetch the `@ai-sdk/openai-compatible` provider package on first use —
  needs egress (or a warmed cache) in-cluster. To validate when wiring the eaig creds.
- Full review e2e needs a live LLM (the eaig gateway), so it is **not** exercised in CI; the config
  generation, MCP wiring (vs real OpenCode), and result parsing are.

## Alternatives rejected

- **Drive OpenCode over ACP** (the design doc) — needs a Rust ACP client; `run` is the headless path.
- **Build our own agent loop** — re-implements tool-calling/streaming/provider support OpenCode
  already has.
- **A default model baked in** — rejected for the same reason as embeddings (ADR-0018): fail loud on
  misconfig; the operator picks the model.

## Amendment (2026-06-20): PR-scoped, diff-grounded prompt

The first live review audited the whole repository and surfaced findings on files the PR didn't
touch (scope leak), with no concrete fixes. The prompt is now a **pull-request** review:

- The runner computes `git diff <base>..<head>` over the checkout (best-effort; `clone::pr_diff`)
  and pastes the changed-file list + unified diff into the prompt. The agent is told to raise
  findings **only on lines the diff changes**, using the repo + MCP tools as impact context.
- The `summary` is shaped around the AI-governance lens: intent/scope · correctness · security ·
  tests. Severities are `error|warning|info`.
- `ReviewFinding` gains an optional `suggestion` (exact replacement for the line) so the model can
  offer a concrete fix; slice 6 (ADR-0022) renders it as a committable GitHub ```suggestion block.
- No diff available (non-PR run, or base commit unfetched) → falls back to an unscoped review.

## Amendment (2026-06-20): configurable prompt + tunable Job deadline

The prompt is now two parts: a **default guidance** (`DEFAULT_REVIEW_GUIDANCE`) tuned for high-signal,
*terse, human-skimmable* output (report only what matters, ≤8-word titles, 1–2-sentence bodies,
silence over noise), and a **fixed output contract** (`OUTPUT_CONTRACT`) carrying the scope rule +
JSON shape + suggestion format. Operators override only the guidance via **`REVIEW_SYSTEM_PROMPT`**
(plumbed: dispatcher env → injected into the agent Job → `ReviewConfig`); the contract is always
appended last, so an override tunes behaviour without ever breaking parsing or diff-scoping.

Separately, the agent Job's runtime cap is now **`AGENT_JOB_DEADLINE_SECONDS`** (dispatcher env,
default 3600; a bad/zero value falls back to the default rather than disabling the cap), replacing the
hardcoded 1h from #51.
