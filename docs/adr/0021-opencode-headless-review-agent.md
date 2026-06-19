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
