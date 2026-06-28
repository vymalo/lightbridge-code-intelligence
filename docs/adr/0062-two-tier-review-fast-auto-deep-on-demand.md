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
- **Per-tier config — independent blocks (amended 2026-06-27).** This ADR originally said "one model,
  two loop shapes" — vary tools/effort/turns, never the model. **Superseded in practice:** the operator
  wants a cheap fast model + a strong deep model (e.g. GLM-5.2 on `@mention`), so each tier gets a
  **fully-independent config block** (`review.fast` / `review.deep` in ai-helm-values — own model,
  gateway, prompt, reasoning budget, timeout). The runner accepts BOTH the flat `review.*` (legacy: both
  tiers share it) and the nested blocks, so it deploys before the values are restructured (transition-
  safe, `deny_unknown_fields`). The structural fast behavior (single diff-only turn, no retrieval) is
  still keyed on the tier, independent of which model the fast block names.
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

## Amendment (2026-06-28) — fast-tier hardening: dedicated prompt, config-driven tools, real-handle framing

Live dogfood of the first fast-tier rollout (vymalo-shop #303/#304/#305) worked end-to-end — findings
posted inline, no retrieval leaked — but surfaced three rough edges, all rooted in the fast tier **reusing
the deep system prompt**. The deep prompt tells the model to *investigate first* (search / graph /
`read_file`), so on a tier where those tools don't exist, M2.7 opened each run by **calling tools that get
refused** (turns 0–2 on #304), only then reviewed the diff, and so **ran out of its turn budget before
`finish`** — landing in the `Exhausted` backstop every time instead of producing a clean verdict. Two
consequences followed: (a) the exhausted-pass note was a generic banner that **didn't acknowledge the
findings** the run had actually posted, and it hardcoded the wrong **`@lightbridge`** handle (the real App
is `lightbridge-assistant`); (b) without repo access the model **over-rated unverifiable concerns as P0/P1**
(a client-side Flutter route "auth" P1 on #303 that a client route cannot actually gate).

Three changes, keeping the ADR's decision intact:

1. **Dedicated fast system prompt** (`config.reviewSystemPromptFast` → `review-system-fast.md`, pointed at
   by `review.fast.system_prompt_file`). It never mentions retrieval/`read_file`, tells the model to review
   the diff directly, record findings, and **always `finish` with a verdict** (even if clean), and — the
   calibration fix — to **raise only what the diff proves**, phrasing the unverifiable as a P2 question, not
   a P0/P1. The deep tier keeps the full `reviewSystemPrompt`.
2. **Config-driven per-tier tool allowlist** (`review.<tier>.tools`). The exact tool names a tier offers are
   now declared in `ai-helm-values` (fast = `[add_review_comment, finish, abort]`) instead of relying on the
   runner's hardcoded wind-down set. The runner validates every name against the known surface and **fails
   the review closed** on an unknown one; the fast-tier non-offered-tool refusal guard still backstops a
   hallucinated call. (Tools were already hidden from fast mode — this externalizes the set so an operator
   tunes each tier from the ConfigMap.)
3. **Control-plane-owned fast framing.** The "🅵 quick pass — mention @handle for a deeper review" body is
   now rendered at `finalize_review` (`render_fast_body`), where the **real** App handle lives
   (`GITHUB_APP_HANDLE`), instead of by the runner (which couldn't know it). Keyed on the task `tier`, it
   marks **every** fast review as a quick pass (a blockquote banner distinct from the deep review's heading),
   appends the model's verdict when present, and posts the inline findings either way. The runner no longer
   sets a fast summary.

**Deploy ordering** (the [`deny_unknown_fields`](0021-file-based-config.md) rule): ship the **runner image**
carrying `review.<tier>.tools` first, then the **ai-helm** chart (renders the field + the second prompt
file), then the **ai-helm-values** that set them. The fast prompt alone needs no new runner (it rides the
existing `system_prompt_file`); only the `tools` field gates on the new image.
