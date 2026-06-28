# ADR-0040: A re-review reads the agent's own prior review as context

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) reviews a PR's diff fresh
on every run. A re-review — whether triggered by a new push or by an `@mention … review again`
([#168](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/168)) — gets the diff, the
maintainer's command, and the repo's instruction files ([ADR-0036](0036-auto-read-agent-instruction-files.md)),
**but nothing about its own earlier reviews of the same PR**.

Observed live on `vymalo/vymalo-shop#241` (two runs on the same PR, same file):

1. Run 1 found a `P1/quality` IndexedDB connection leak in `tx()`.
2. Run 2 (a `review again`) found a *different* `P1/correctness` bug — and its summary **flatly
   contradicted run 1**: it praised the persistence layer as a *"carefully designed lazy singleton
   connection,"* exactly the code run 1 had flagged as *"opens a new connection on every call and never
   closes it."*

Because the agent has no memory of its prior output, each run finds one issue, non-deterministically,
and a later run can confidently bless what an earlier run condemned. The union of findings only emerges
across runs, and the self-contradiction actively **erodes trust** — the explicit goal of epic #137 is a
*trustworthy* reviewer, and a reviewer that disagrees with itself on the record is worse than one that
merely misses things.

## Decision Drivers

- **Reconcile, don't reset.** A re-review should build on its prior verdict — confirm a finding is
  resolved by the new diff, or restate it — never silently contradict it.
- **No new GitHub round-trip.** We already persist each posted review's summary + findings +
  `github_review_id` in our own `reviews` table ([ADR-0035](0035-review-feedback-signal.md) /
  [#144](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/144)). The prior context is a
  local query, not a GitHub fetch.
- **Trust boundary** ([ADR-0002]). The control plane owns GitHub + the database; the runner is a pod
  with no App key. The prior context must be assembled control-plane-side and handed to the runner, the
  same way the diff and token already are.
- **Fail open to the old behavior.** A missing/failed lookup, an older control plane, or a first review
  must degrade to exactly today's blind re-review — never a failed task.
- **Bounded prompt cost.** The injected block must stay small regardless of how many findings a PR has
  accumulated over many runs.

## Decision

On a `review`-kind task, the control plane looks up the **most recent prior review of the same target**
(`reviews ⋈ tasks` on `(repository_id, target_type, target_id)`, excluding the current task), formats
its verdict + findings into a compact context block, and returns it as a new optional
`prior_reviews` field on `TaskContextResponse`. The runner injects that block into the user prompt
(after the diff, before the repo's own instructions) with a reconcile instruction: *for each prior
finding, confirm the diff resolves it or restate it; do not contradict a prior conclusion without
saying what changed.*

Scope and safety:

- **`review` kind only.** An `ask` reply or an `index` run has nothing to reconcile.
- **Latest review only**, findings capped (`PRIOR_FINDINGS_CAP = 30`), titles not bodies — the block
  stays small. The newest review is the relevant one; older runs' findings either persisted into it or
  are stale.
- **Best-effort.** A query error logs a warning and yields `None`; the field is `#[serde(default)]` on
  the runner side, so an older control plane / a first review reads exactly as before.
- **Not authoritative.** The block sits in the user message; the machine tool-protocol in the system
  message stays the authoritative instruction. The prior review is our own output, but it is fed as
  context to reconcile against, not as ground truth.

## Consequences

- **Good:** re-reviews stop contradicting themselves; findings accumulate across runs instead of
  resetting; no extra GitHub API calls; no config change to deploy.
- **Cost:** a small, bounded prompt-size increase on re-reviews (none on a first review); one extra DB
  query per `review` task context fetch.
- **Limitation:** this addresses the *consistency* of a re-review against the previous one. It does
  **not** by itself make a single run cover the whole diff — that is the separate coverage concern
  (B, #137). A and B compose: B widens a single run's coverage; A keeps successive runs coherent.
- **Future:** if the bimodal latency continues to make one-finding runs common, we may carry more than
  the single latest review, or summarize across all prior reviews of the target.

## References

- Epic [#137](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/137) — trustworthy
  review agent v2.
- [ADR-0035](0035-review-feedback-signal.md) — persisted review record (the data source).
- [ADR-0036](0036-auto-read-agent-instruction-files.md) — the sibling "inject context into the prompt"
  mechanism this mirrors.
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the agent loop + prompt assembly.
