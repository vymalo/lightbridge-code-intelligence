# ADR-0062: Two-tier review — a fast auto pass on every PR, a deep review on demand

- **Status:** Proposed
- **Date:** 2026-06-27
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) runs one heavyweight loop for
**every** trigger: clone → (reuse index) → risk-first investigation over graph/vector retrieval +
`read_file` → verification/refute → grouped review. On a real repo with `reasoning_effort` set, that is a
**multi-minute-to-~25-minute** job. It produces excellent, repo-aware reviews — but running it
automatically on **every** opened PR is too slow and too costly for the signal most PRs need (a trivial
version-bump pays the same tax as a subtle concurrency change).

This session pinned the cost precisely — and it is **not** the model or the gateway:

- Same Envoy gateway, same DeepSeek-V4-Flash, local repro: `reasoning_effort` `none → medium` took a
  review from **1m38s → 4m** ([ADR-0060](0060-capture-model-reasoning-and-glm-5-2-latency-finding.md)).
  The dominant terms are **`reasoning_effort` + turn count + retrieval depth**, not the model id and not
  gateway contention.
- So the lever to make reviews cheap is the **loop shape** (tools / effort / turns / timeout), not a
  model swap. Swapping models is actively *harmful* as a per-tier lever: `stream`/timeout/budget are all
  coupled to the model (ADR-0060), so two models = double the coupling-maintenance trap.

We also now have a **deterministic, near-instant** finding source — SAST via opengrep
([ADR-0061](0061-sast-deterministic-finding-source.md)) — that needs no LLM and no retrieval.

## Decision

**Split review into two tiers, keyed solely by the trigger. One model, two loop shapes.**

### Fast tier — automatic, on `pull_request opened` (PR targets only)
- **SAST (opengrep) is the backbone** — deterministic, no LLM, ~instant.
- **Plus exactly ONE diff-only LLM turn** — no retrieval tools registered (no graph/vector), **no agentic
  loop** (a hard 1-turn cap, not just a low `max_turns`), `reasoning_effort` none/low, short request +
  job timeout. It turns the SAST findings + the raw diff into a human-readable verdict and a cheap
  logic/quality sanity-check.
- Output: ONE grouped PR review (SAST findings + the single-turn verdict), single-channel per
  [ADR-0056](0056-control-plane-owns-the-posted-output.md).
- Target wall-clock: **≲ 2 min**.

### Deep tier — manual, on any `@mention`
- The current heavyweight loop, unchanged: full graph + vector retrieval, `read_file`, `reasoning_effort`
  medium, generous `max_turns`, **streaming on so the per-chunk idle timeout governs**, and a **long job
  timeout (2h is acceptable)** — it is user-requested and async, so it can take its time.
- On a **PR** → a deep repo-aware review. On an **issue** → a conversational answer (the
  [ADR-0033](0033-inbound-command-parsing-and-run-kinds.md) issue/answer path is **retained**). The
  `@mention` body is free-form; deep mode handles it per target.

### Cross-cutting
- **One model for both tiers.** Tiers differ in tool-set + `reasoning_effort` + `max_turns` + timeout,
  **never** the model. (A per-tier model override stays *possible* via the ADR-0051 config machinery, but
  is not the default lever and is discouraged.)
- **Same system prompt for both.** The persona/standards are constant; only the toolset and budget
  differ. Caveat: the prompt's "how you investigate" section assumes retrieval — the fast tier simply
  does not register those tools (the model only ever sees the tools it has), and the fast-tier prompt
  must not *promise* tools it lacks. The factual `TOOL_PROTOCOL` (code) already varies by registered
  tools.
- **The "we're on it" acknowledgement is comment-free.** A **👀 reaction** on the trigger plus a **Check
  Run** ("Lightbridge — reviewing…" → "done / N findings"), using the existing `Checks: Read/Write`
  permission. A status *comment* is explicitly rejected — it would re-introduce the multi-channel clutter
  ADR-0056 / #226 just removed.

### Mechanism
- A **`tier`** (`fast` | `deep`) on the task, derived from the trigger: `pull_request opened` → `fast`;
  `@mention` → `deep`. Carried in the task context to the runner.
- **Per-tier runner config** (enabled tool-set, `reasoning_effort`, `max_turns`, request timeout, job
  `activeDeadlineSeconds`) — two blocks in `ai-helm-values`, resolved by the runner from the task's tier,
  reusing the [ADR-0051](0051-per-model-config.md) per-config machinery.
- The fast tier runs SAST, registers no retrieval tools, executes a single capped LLM turn, and
  finalizes — it never enters the investigation/verification loop.

## Consequences

- **Good:** every PR gets a sub-2-min deterministic + light-LLM signal; the expensive deep review is
  deliberate and on-demand, so cost is bounded (no 25-min job per push); the 2h ceiling is safe because
  it applies only to the user-requested deep tier.
- **The fast tier will miss logic bugs that need repo context** — by design. Set expectations in the
  Check Run / verdict ("fast pass — `@lightbridge review` for a deep, repo-aware review").
- **Two code paths** (fast vs deep) in the runner; modest added complexity, mitigated by sharing
  everything except the registered tool-set and the budget. SAST is already wired (ADR-0061), so the fast
  tier is mostly "constrain the existing loop + always run SAST + single-turn cap."
- **Per-PR LLM cost is non-zero** (one short turn each) — the price of a readable verdict over pure SAST.
  (SAST-only auto was considered and rejected: it gives no logic/quality read and no human verdict.)
- Keeps the issue/answer surface (ADR-0033) and the single-channel PR output (ADR-0056) intact.

## Alternatives considered

- **Weaker model on auto, stronger on manual.** Rejected — the model isn't the cost driver, and a
  per-tier model doubles the `stream`/timeout/budget coupling burden (ADR-0060).
- **SAST-only fast tier (no LLM).** Cheapest, fully deterministic, but no logic/quality signal and no
  human-readable verdict on auto. Rejected in favour of SAST + one capped turn.
- **A "reviewing…" status comment.** Rejected — re-introduces the multi-channel clutter ADR-0056 / #226
  removed; a reaction + Check Run conveys the same with no comment noise.
- **Disable auto review entirely (manual-only).** Considered; rejected because a cheap deterministic
  auto signal on every PR is worth keeping once it no longer costs a full deep review.

## References

- [ADR-0033](0033-inbound-command-parsing-and-run-kinds.md) — run kinds + targets (issue/answer path retained).
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the mediated-tools agent loop both tiers share.
- [ADR-0039](0039-agent-llm-resilience-and-observability.md) — timeouts/streaming the deep tier relies on.
- [ADR-0051](0051-per-model-config.md) — per-config machinery reused for per-tier config.
- [ADR-0056](0056-control-plane-owns-the-posted-output.md) — single-channel PR output the ack must not break.
- [ADR-0060](0060-capture-model-reasoning-and-glm-5-2-latency-finding.md) — the cost diagnosis (effort/turns/retrieval, not model/gateway).
- [ADR-0061](0061-sast-deterministic-finding-source.md) — SAST/opengrep, the fast tier's backbone.
