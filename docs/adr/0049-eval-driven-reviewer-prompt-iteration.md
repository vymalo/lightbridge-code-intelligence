# ADR-0049: Eval-driven reviewer-prompt iteration — golden cases before deploy

- **Status:** Proposed
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning

## Context and Problem Statement

Every meaningful reviewer-prompt change so far (ADR-0040 through ADR-0045, the 2026-06-24 "surface
findings at all priorities" fix, and the two prompt ADRs proposed alongside this one,
[ADR-0047](0047-review-prompt-grounding-and-uncertainty.md) /
[ADR-0048](0048-review-prompt-structure-and-technique.md)) has been validated **by dogfooding in
production** — ship the prompt to `ai-helm-values`, watch the next live PR review, fix forward. That
loop has worked, but it is *vibe-based evaluation*: changes are judged on a handful of live runs, there
is no regression guard, and a prompt edit that fixes failure A can silently reintroduce failure B with
no signal until a human notices it on a real PR. The empty-retrieval hallucination on PR #187 is the
cost of that loop — a regression that reached production before anyone saw it.

OpenAI's evaluation guidance names this anti-pattern directly and prescribes the fix: **"Adopt
eval-driven development: evaluate early and often,"** and treats "vibe-based evals" as the thing to
move *away* from. Anthropic's *Building effective agents* makes the same point structurally with the
evaluator-optimizer pattern (generate → evaluate against clear criteria → refine in a loop). We have the
raw material to do this: a deterministic agent loop with a fully mockable boundary.

This is epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).

## Decision Drivers

- **Stop shipping prompt regressions to prod.** A prompt change should pass a known set of failure-mode
  cases *before* it reaches a live PR.
- **The failure modes are concrete and enumerable** — empty-retrieval hallucination, out-of-scope
  findings, P2-in-prose, non-convergence, anchoring, self-contradiction. Each is a testable assertion.
- **The loop is already deterministically mockable.** `run_native_agent` takes a `ChatClient` over a
  base URL and an `EmbeddingsClient` / `ControlPlaneClient` over base URLs; the existing unit tests
  (agent.rs) already drive the whole loop against a `wiremock` `MockServer` with scripted chat
  responses and assert on the `ReviewOutcome` and the buffered actions. The harness is a generalization
  of machinery that exists.
- **Cheap and offline first.** The first tier asserts on the *loop's behaviour* with a scripted model
  (no gateway, no tokens, runs in CI). A second, optional tier runs the *real* GLM model against golden
  diffs for prompt-quality signal — gated/manual because it costs tokens and is non-deterministic.

## Decision

Introduce an **eval-driven workflow** for reviewer-prompt changes, in two tiers.

### Tier 1 — offline golden cases (deterministic, in CI)

A small golden-case suite that drives `run_native_agent` against a scripted chat endpoint (extending the
existing `Script` / `mount_chat` test harness in `agent.rs`) and asserts on observable behaviour. Each
case encodes one observed failure mode as a pass/fail check on what the loop buffers and how it ends.
Proposed seed cases, one per failure mode from epic #177:

1. **Empty-retrieval grounding (ADR-0047).** Mock all retrieval to return `[]`. Script a model that, if
   the prompt is wrong, would assert "no references — feature removed." **Assert** the run does not
   buffer or `finish` with an absence/removal claim, and that it falls back to `read_file` or records
   "could not verify." (This is the #187 regression, frozen as a test.)
2. **Out-of-scope finding (#3).** Provide a diff touching `a.rs`; script a model that tries to
   `add_review_comment` on `b.rs` (unchanged). The control plane validates write-back (ADR-0022) and
   the per-call mediated tool path anchors/rejects against the diff (ADR-0037) — that server side is the
   enforcement. This eval's distinct value is the *prompt's* effect: run the agent **with** the
   review-only-the-diff rule (the post-0047/0048 state) and **assert the agent does not attempt** the
   out-of-scope call in the first place — i.e. the prompt prevents the attempt, not merely the server
   rejecting a stray one. (A negative-control run against a prompt missing the rule shows the delta.)
3. **P2 recorded, not narrated (#2).** Script a P2-worthy issue. **Assert** it is recorded via
   `add_review_comment`, not only mentioned in the `finish` summary; assert the summary does not reduce
   to "no P0/P1 findings."
4. **Convergence (#4).** Script a model that wanders. **Assert** the wind-down/coverage/budget machinery
   forces a `finish`/`Exhausted` within budget and that buffered findings are still finalized
   (ADR-0042/0045 already have partial coverage here — fold it into the suite).
5. **Anchoring (#5).** Script a finding on a real line that isn't an added/changed line. **Assert** the
   expected handling (bucketed/deferred, not a bogus inline anchor).
6. **Self-consistency (#6).** Provide a `prior_reviews` block; script a model. **Assert** the run
   reconciles rather than contradicts (ADR-0040 behaviour, frozen).

These are **behavioural** assertions on the loop, not LLM-quality judgments — they are deterministic and
belong in `cargo nextest`. They guard the *machinery* and the *prompt's structured effects*; they do not
prove the live model writes good prose.

### Tier 2 — live golden diffs (manual / gated, real model)

A separate, opt-in harness (an `xtask` or a manually-run integration test, not in the default CI gate)
that runs the **real** `adorsys-reviewer` model against a small set of curated golden diffs — each a real
past PR with a known expected outcome (e.g. the #187 hollow-index scenario, a PR with a planted P0, a
clean PR that should pass). Output is scored by a rubric (did it find the planted bug? did it stay in
scope? did it hallucinate on the empty index?), optionally LLM-judged. This is the *quality* signal; it
is non-deterministic and costs tokens, so it runs on demand before a prompt deploy, not on every commit.
*Source:* OpenAI evals — describe task → run with test inputs → analyze and iterate.

### Workflow contract

A change to `config.reviewSystemPrompt` (or to the prompt-shaping ADRs 0047/0048) is **proposed with a
matching Tier-1 golden case** and SHOULD be checked against Tier 2 before the operator deploys it to
`ai-helm-values`. This turns the dogfood loop from "ship and watch" into "prove, then ship, then watch."

## Consequences

- **Good:** prompt regressions (the #187 class) are caught offline, in CI, before they reach a live PR.
  Each observed failure mode becomes a permanent regression guard. The workflow gives ADR-0047 and
  ADR-0048 a concrete acceptance test instead of a hope.
- **Good:** Tier 1 reuses the existing `wiremock`-based loop tests — low build cost, fully deterministic.
- **Cost / limits:** Tier 1 asserts on *loop behaviour with a scripted model*, not on real-model prose
  quality — a scripted case proves the machinery responds correctly, not that GLM will choose the right
  action. Tier 2 closes that gap but is non-deterministic and token-costly, so it stays manual/gated.
  Maintaining golden diffs has upkeep as the codebase and prompt evolve.
- **Scope:** this ADR proposes the *workflow and the seed case list*; building the harness is
  implementation under #177. It deploys nothing.

## What this deliberately defers

- **A standing CI job that calls the real gateway** — flaky and token-costly; Tier 2 stays manual until
  there is a stable, cheap eval model budget.
- **Automated LLM-as-judge scoring in CI** — useful for Tier 2 scoring but adds its own
  calibration/cost concerns; start with a human rubric.
- **A public eval dataset / leaderboard** — out of scope; these golden cases are internal regression
  guards.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0047](0047-review-prompt-grounding-and-uncertainty.md),
  [ADR-0048](0048-review-prompt-structure-and-technique.md) — the prompt changes this gates.
- [ADR-0013](0013-local-dev-and-build-tooling.md) — nextest + wiremock, the existing test substrate
  Tier 1 extends.
- [ADR-0022](0022-review-writeback-control-plane.md) — control-plane validates review write-back.
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the per-call mediated tool path that anchors/rejects
  against the diff (the server-side enforcement the out-of-scope eval distinguishes from the prompt effect).
- [ADR-0034](0034-agent-run-transcript-and-observability.md) — the transcript, a data source for
  scoring Tier 2 runs.
- [ADR-0040](0040-re-review-reads-prior-findings.md) — prior-review reconciliation (the self-consistency
  golden case).
- [ADR-0042](0042-risk-first-review-and-parallel-batching.md),
  [ADR-0045](0045-context-window-budget.md) — the budgets/wind-down the convergence golden case exercises.
- OpenAI, *Evaluation best practices* — "Adopt eval-driven development: evaluate early and often"; the
  "vibe-based evals" anti-pattern: https://developers.openai.com/api/docs/guides/evaluation-best-practices
- OpenAI, *Evals guide* — describe → run → analyze/iterate loop:
  https://developers.openai.com/api/docs/guides/evals
- Anthropic, *Building effective agents* — the evaluator-optimizer pattern (generate → evaluate against
  clear criteria → refine): https://www.anthropic.com/engineering/building-effective-agents
