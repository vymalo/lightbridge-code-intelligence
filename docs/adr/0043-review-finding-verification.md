# ADR-0043: Finding verification — evidence citation + a refute pass (Phase 2)

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** @stephane-segning

## Context and Problem Statement

After Phase 1 (ADR-0042) the review agent is faster and more strategic, but a check of five live
reviews exposed the real quality gap: **confidently-wrong P0/P1 findings**. Concretely, a review of a
runner PR asserted a config helper "silently ignores `LLM_MAX_BATCH_SIZE`" as a P1 — verified false
(the code consults the env). A wrong blocker costs more trust than a missed nit.

The originally-planned fix was a self-reported `confidence` field + a `minConfidence` filter. The check
showed why that is insufficient: **a model that is wrong is also confident**, so it would self-label the
false finding "high" and the filter would pass it. What kills a confident-wrong finding is
*verification* — forcing the claim to be checked against the actual code.

This is epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177), Phase 2.

## Decision Drivers

- **Kill confidently-wrong P0/P1s** — the measured gap.
- **Verification gated by cost** (the chosen shape): cite evidence always; spend the extra refute turn
  only on P0/P1, where a false positive hurts most.
- **No DB migration** if avoidable — ship safely in one push.
- **Rollout-safe**: a runner image that lands before the evidence-aware prompt must not silently drop
  findings.
- **Reuse existing levers** (the one-shot bounce pattern from ADR-0041; last-write-wins buffer).

## Decision

1. **Evidence citation (always).** `add_review_comment` gains an `evidence` field — the exact lines /
   symbol the finding rests on. It is folded into the rendered body so the proof is visible to the human
   and stored with the finding (no schema/DB change). The tool description and the operator prompt
   require it ("if you can't cite it, don't record it"); **parsing keeps it optional** so a runner that
   ships ahead of the prompt still records findings (rollout safety).

2. **A refute pass (the false-positive killer).** Before the first `finish` with any P0/P1 finding
   recorded, the loop **bounces once** and instructs the model to re-verify each P0/P1 against its cited
   evidence — looking at the real code, not memory — and to `retract_finding(file, line)` any whose
   claim does not hold. One-shot (adds at most one turn), cost-gated to P0/P1.

3. **`retract_finding` tool + path.** A new mediated tool deletes a buffered inline finding by
   `(file, line)` via a new internal endpoint (`POST …/review/inline/retract`) →
   `db::delete_pending_inline` (a delete on the existing `pending_review_actions` table — **no
   migration**). Kept available in the wind-down tool set so the refute pass works even late.

## Consequences

- **Good:** a confidently-wrong P0/P1 now faces a "prove it against the cited lines or retract it" turn
  — exactly what would have caught the live false positive. Evidence in the body also makes *human*
  verification faster. No migration; rollout-safe; reuses the bounce + buffer machinery.
- **Cost:** at most one extra turn per run that has P0/P1 findings. The refute is model-driven, so it is
  not a hard guarantee — a determined-wrong model could re-affirm — but requiring it to re-examine the
  cited evidence is a materially stronger gate than a self-confidence label, at far lower cost than an
  independent multi-agent verifier.

## What this deliberately defers

- **A structured `confidence` column + `minConfidence` filter** — superseded for the quality goal by the
  refute pass; revisit only if confident-wrong findings persist after this lands. (Would need a DB
  migration.)
- **Finding caps (`maxPublishedFindings` + `overCapPolicy`)** — live reviews post 1–3 findings, so
  volume is not the active problem; a finalize-side cap is a cheap follow-on if that changes.
- **The `record_risk_map` tool** — observability/enforcement of the risk-first plan; valuable but not
  on the quality-gap critical path. Follow-on.

These are tracked under epic #177; this ADR ships the verification core that addresses the measured
problem.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177); [ADR-0042](0042-risk-first-review-and-parallel-batching.md) (Phase 1).
- [ADR-0041](0041-full-diff-coverage-gate.md) — the one-shot bounce pattern reused here.
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — mediated tools + the pending buffer.
