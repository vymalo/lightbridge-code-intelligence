# ADR-0026: Native Rust review agent with structured tool calls (supersede OpenCode)

- **Status:** Accepted (supersedes [ADR-0021](0021-opencode-headless-review-agent.md))
- **Date:** 2026-06-21

## Context and Problem Statement

[ADR-0021](0021-opencode-headless-review-agent.md) used **OpenCode** (`opencode run`, headless) as the
review agent: the runner spawns it with a generated `opencode.json` wiring the eaig OpenAI-compatible
provider + our two stdio MCP servers, the agent reasons over the repo, and **emits the review as a
fenced ` ```json ` block on stdout** which the runner scrapes (`parse_review` → `last_json_block`).

Two problems have shown up in production:

- **Fragile output.** We parse free-text stdout. When the model is chatty, truncates, or doesn't emit
  a clean final block, parsing fails — observed: `review failed (non-fatal): parsing the review result
  from opencode output`. The review (our headline feature) silently degrades.
- **Opaque control.** OpenCode is an external Bun/Node binary in the runner image. We can't easily give
  the agent **our own control tools** (submit findings, report progress, abort), tune retries/context
  budget, or react to cancellation from inside the loop — the loop isn't ours.

## Decision Drivers

- **Robust I/O:** the review result should be validated at a **tool boundary**, never scraped from
  stdout (the same property that makes our other structured calls reliable).
- **Control:** own the agent loop — tool dispatch, retries, context/token budget, and cooperative
  cancellation (complements the runner self-cancel poll, ADR-0024/#116).
- **Fewer moving parts:** drop OpenCode + the Bun/Node runtime from the runner image.
- Reuse what we have: the MCP servers are already thin clients of the control-plane retrieval API
  ([ADR-0020](0020-mcp-servers-via-control-plane.md)); the control plane still validates findings
  against the diff and writes back ([ADR-0022](0022-review-writeback-control-plane.md)) — unchanged.

## Decision Outcome

**Replace OpenCode with a native Rust agent loop in the runner.** It calls the eaig OpenAI-compatible
**Chat Completions** endpoint with **function/tool calling**, and exposes our capabilities as tools the
model invokes directly:

- **Retrieval tools** (the current MCP surface): `vector_semantic_search`, `graph_find_symbol`,
  `graph_get_callers` — implemented as direct calls to the control-plane internal retrieval API (the
  MCP servers were already proxies to it), so the review agent needs no separate MCP subprocesses.
- **Control tools** (new): `submit_findings(summary, findings[])` — the agent returns the review by
  **calling this tool**; the payload is deserialized + validated here, replacing stdout parsing.
  `report_progress(note)` for observability, and `abort(reason)` so the agent can bail cleanly when it
  can't produce a useful review (recorded, not a crash).

The loop: system+diff prompt → model → (tool calls: search/graph) → … → `submit_findings`. The control
plane still re-validates every finding against the PR diff before write-back (ADR-0022).

> MCP framing: we keep the **tool contract** (the same tool names/semantics the MCP servers exposed),
> but the review agent dispatches them in-process instead of over stdio MCP. External MCP consumers, if
> any, can keep using the standalone servers; the review path no longer depends on them.

### Migration (phased — don't break the working path)

1. Build the native loop behind a flag (e.g. `REVIEW_AGENT=native|opencode`), default `opencode`.
2. Reach parity (tool calls, suggestions, resources, the ADR-0024/#103 finding format) and dogfood.
3. Flip the default to `native`; remove OpenCode + Bun from the image and `parse_review`/`opencode.json`.

### Consequences

- Good: no more stdout-parse failures (validated tool payload); full control over retries, budget, and
  cancellation; smaller runner image; the agent gains real control tools.
- Bad: we own an agent loop now — the multi-turn tool-call protocol, token budgeting, and provider
  quirks that OpenCode handled for us. More code + tests to maintain.
- Neutral: the control plane's validation + write-back (ADR-0022) and the finding format (#103) are
  unchanged — this swaps *how the review is produced*, not what's posted.

### References

- Supersedes [ADR-0021](0021-opencode-headless-review-agent.md). Builds on
  [ADR-0018](0018-openai-compatible-embeddings.md) (OpenAI-compatible provider),
  [ADR-0020](0020-mcp-servers-via-control-plane.md) (tools = control-plane clients),
  [ADR-0022](0022-review-writeback-control-plane.md) (diff validation + write-back).
- Symptom: `agent_runner` log `review failed (non-fatal): parsing the review result from opencode
  output`. Current code: `services/agent-runner/src/review/{mod,parse,config}.rs`.
