# ADR-0065: Re-review must not duplicate — dedup on unchanged commits, reconcile by re-deriving

- **Status:** Proposed
- **Date:** 2026-06-28
- **Deciders:** @stephane-segning

## Context and Problem Statement

[ADR-0040](0040-re-review-reads-prior-findings.md) feeds the agent its **prior review** on a re-run so it
**reconciles** instead of contradicting itself across runs (the original failure: two runs found different
P1s and the second flatly contradicted the first). That fixed contradiction — but a re-review on an
**unchanged commit** now produces a **carbon copy**.

Live evidence (webank-mobile#112): an `@mention` re-review ran on the **same `head_sha` (`f4c8411`)** as the
prior review 33h earlier and re-posted the **same 5 inline findings on the same 5 files**, titles differing
only in trivial wording. The verdict was honest (*"re-verified both P1s — both hold"*) but the PR was left
with **10 inline comments — 2× each identical finding**. Zero new information, pure noise, and a full
(expensive) agent run spent to reproduce a result.

How should a re-review behave when the code hasn't meaningfully changed?

## Decision Drivers

- **No duplicate comments.** Re-running must never leave two identical inline findings on a PR.
- **Keep the anti-contradiction win** of [ADR-0040](0040-re-review-reads-prior-findings.md).
- **Don't anchor.** *"Reconcile, don't contradict"* biases toward **restating** — it can re-emit a prior
  **false positive** unchecked (seen: *"persists from prior review"* on vymalo-shop#303–305) and suppress
  genuinely-new angles.
- **Cost.** A re-review of unchanged code shouldn't pay for a full retrieval+multi-turn run.
- The control plane already owns the posted output and persists the prior review's comments + findings
  ([ADR-0056](0056-control-plane-owns-the-posted-output.md)), so dedup has the data it needs.

## Considered Options

- **Option A — Unchanged-`head_sha` short-circuit.** If an `@mention` review targets the same `head_sha` as
  the last posted review (no new commits), **don't run the full agent**: post a terse verdict — *"no new
  commits since my last review (<link>); the N findings still stand"* — with an explicit keyword to force a
  full re-run.
- **Option B — Dedup at finalize.** Always run, but before posting, **drop inline findings byte-identical
  (normalized `(file, line, body)`) to the prior review's** and post only new/changed findings + a
  reconciliation note. Handles the "new commits, partial overlap" case too.
- **Option C — Re-derive-then-reconcile prompt.** Change the [ADR-0040](0040-re-review-reads-prior-findings.md)
  injection from *"reconcile, don't contradict"* to *"re-derive independently first, then reconcile — and
  **retract** any prior finding that no longer holds."* Orthogonal to A/B; addresses anchoring.
- **Option D — Status quo.** Full re-review every time (the duplicate above).

## Decision Outcome

Chosen: **A + B + C together.** **A** avoids the wasted run and the duplicate in the common case (re-review,
no new commits). **B** is the correctness backstop for when a run *does* happen (new commits with overlapping
findings) — the control plane dedups inline comments against the prior review so a finding is never posted
twice. **C** keeps [ADR-0040](0040-re-review-reads-prior-findings.md)'s anti-contradiction value while
breaking the echo: the agent re-derives, then reconciles, and actively **retracts** stale findings. This
ADR **refines ADR-0040**, it does not revert it — prior reviews are still injected; their *output* is now
constrained.

**Proposed** — open for discussion, especially the force-re-run keyword (A) and the dedup key (B).

### Consequences

- **Good** — a re-review on an unchanged commit posts a one-line verdict + a link, not 5 duplicate comments.
- **Good** — overlapping findings across commits are deduped, not stacked.
- **Good** — the agent can drop a prior false positive (C), which "reconcile, don't contradict" discouraged.
- **Good** — saves a full agent run when nothing changed (A).
- **Bad / watch** — "meaningfully changed" is `head_sha`-based; a force-rerun keyword is needed for "re-review
  anyway" (e.g. after a model/prompt change). The dedup key (B) must normalize whitespace/wording so trivial
  re-phrasings still match.
- **Neutral** — needs the prior review's `head_sha` + its posted comments, both already persisted.

## Pros and Cons of the Options

### A — unchanged-head short-circuit
- Good — cheapest; kills the common-case duplicate before any cost.
- Bad — needs a force keyword; doesn't help the new-commit-partial-overlap case (that's B).

### B — finalize dedup
- Good — always correct; covers partial overlap across commits.
- Bad — still pays for the run; dedup key must be robust to re-phrasing.

### C — re-derive-then-reconcile prompt
- Good — removes anchoring; lets stale findings be retracted.
- Bad — prompt-only, hard to guarantee; complements but doesn't replace A/B.

### D — status quo
- Bad — the duplicate this ADR exists to remove.

## More Information

- Refines: [ADR-0040](0040-re-review-reads-prior-findings.md) (prior-review injection).
- Lives in: [ADR-0056](0056-control-plane-owns-the-posted-output.md) (the control plane owns the post; dedup goes here).
- Evidence: ADORSYS-GIS/webank-mobile#112 (two identical reviews on `f4c8411`).
- Related: [ADR-0043](0043-review-finding-verification.md) (the refute pass C builds on for retraction).
