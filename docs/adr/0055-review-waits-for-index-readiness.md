# ADR-0055: A review waits for index readiness (the `WaitingForIndex` gate)

- **Status:** Accepted
- **Date:** 2026-06-26
- **Deciders:** @stephane-segning
- **Implements:** [RFC-0002](../rfc/0002-incremental-layered-indexing.md) Phase 1.4 (index-before-review ordering)

## Context and Problem Statement

A review reads the index via the MCP retrieval tools (vector + graph). [ADR-0050](0050-retrieval-pins-to-latest-indexed-snapshot.md) made reviews **reuse** the latest indexed snapshot instead of re-indexing per run; readiness is decided by `db::latest_indexed_commit`:

```sql
SELECT commit_sha FROM code_chunks WHERE repository_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1
```

This returns a commit **as soon as the first chunk row exists** — it conflates *"an index is in progress"* with *"an index is complete."* So a review that fires **while the repo's index is still building** sees `repo_indexed = true`, skips its own indexing, and reuses a **half-written** snapshot → **every retrieval returns 0 hits.**

**Observed live (2026-06-26, dogfood).** A review of `webank-mobile` ran while the repo's *initial* index was still running. Status reported the index as `done`; all searches returned 0 hits; the agent fell back to reading the whole repo by hand. Measured against three reruns of the same repo **after** indexing completed:

| | retracts | runtime | findings |
|---|---|---|---|
| blind (index in-flight) | **7** | ~18 min | 16 recorded → 9 net |
| grounded (index ready) ×3 | **0 / 0 / 0** | ~5–6 min | clean, anchored |

The agent behaved well even blind (the ADR-0047 grounding prompt kept it from hallucinating absence — it read files and posted a real review), but the churn and the ~3× slowdown were **entirely** the missing index. A good reviewer was crippled by a broken precondition with **no signal that anything was wrong** — the log line is identical (`reused base index`) whether the reused index is complete or half-built.

## Decision

**Gate a review (and any non-`index` task) on index readiness using the already-defined-but-unused `WaitingForIndex` task state.**

1. **Enqueue gate.** When a non-`index` task is created (`create_task` / `create_explicit_task`), if an **`index` task is in flight** for the repo (`status IN (queued, running, posting_result, waiting_for_index)`), the task is inserted as **`waiting_for_index`** instead of `queued`. The dispatcher's claim query already only selects `queued`, so a waiting task is simply never claimed.
2. **Release on completion.** When an `index` task reaches a **terminal** status (`set_task_status`), all the repo's `waiting_for_index` tasks are flipped to `queued` and `NOTIFY task_queued` wakes the dispatcher. They then run against the now-**complete** snapshot.

No migration is needed: `tasks.status` is unconstrained `TEXT`, `WaitingForIndex` already exists in the `TaskStatus` enum, and the release reuses the existing `TASK_QUEUED_CHANNEL`.

## Consequences

- **Good:** the observed bug is fixed — a review never starts while the repo's index is in flight, so it always reuses a *complete* snapshot (or, on a truly cold repo with no index running, self-indexes as before). The fix is small, migration-free, and uses the state that was reserved for exactly this.
- **Observability:** the control plane now logs the gate (`review gated: waiting for index`) and the release (`index complete: released N waiting tasks`), so a blind-review window is visible instead of silent.
- **Cost / v1 limits (deliberately scoped):**
  - **Warm reviews wait on a re-index.** While *any* index task is in flight, new reviews wait — even when a *prior* complete snapshot exists that ADR-0050 could have reused. This trades a little latency for never-reads-partial simplicity. Optimizing it (reuse the last *complete* snapshot during a re-index) needs per-snapshot completion tracking — deferred.
  - **Failed/partial index.** Release fires on *any* terminal status (including `failed`), so a failed index that left partial chunks could still be reused by a released review. The robust fix is **populating `repo_index`** (the table already has `status` + `completed_at`, currently dormant — see [RFC-0002](../rfc/0002-incremental-layered-indexing.md)) and gating `latest_indexed_commit` on a `ready` row per commit. Releasing on terminal is chosen over leaving reviews stuck forever; the common case (index succeeds) is fully fixed.
  - **Mid-review re-index race.** A re-index that *starts* after a review is claimed is not covered (the review already passed the gate). Narrow window; same `repo_index`-population follow-up closes it.
- **Supersedes / relates to:** strengthens [ADR-0050](0050-retrieval-pins-to-latest-indexed-snapshot.md) (which assumed the latest snapshot is complete) and is the first wiring of the `WaitingForIndex` state from [RFC-0001](../rfc/0001-horizontally-scalable-control-plane.md)'s queue model.

## References

- [RFC-0002](../rfc/0002-incremental-layered-indexing.md) — Phase 1.4 index-before-review ordering.
- [ADR-0050](0050-retrieval-pins-to-latest-indexed-snapshot.md) — reviews reuse the latest indexed snapshot.
- Dogfood evidence: blind run `b7c98eec` vs grounded reruns `b760a978` / `da0b5511` / `71162299` (same repo).
