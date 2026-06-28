# ADR-0052: Index snapshot pruning — keep the latest + in-flight, sweep the rest

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning

## Context and Problem Statement

Indexing stores a full snapshot per `(repository_id, commit_sha)` in both pgvector (`code_chunks`) and
Neo4j (`Symbol {repo_id, commit, …}`). Since [ADR-0050](0050-retrieval-pins-to-latest-indexed-snapshot.md)
(RFC-0002 Phase 0) a review **reuses the latest indexed snapshot** instead of re-indexing per PR — which
removed the *compute* waste but left the *storage* half untouched: every push to the default branch
(re-index-on-push, #183) writes a **new** full snapshot, and nothing reaps the old ones. `delete_*_for_repo`
only fires on repo disconnect. So a busy repo accumulates one full duplicate index per push, forever, in
both stores — while retrieval only ever reads the newest (`db::latest_indexed_commit`).

We need to bound index storage without breaking in-flight runs, and without yet building RFC-0002's full
`ref`/overlay layer model (Phases 1–2).

## Decision Drivers

- **Bound storage** to roughly one snapshot per repo, not "every commit ever indexed".
- **Never break an in-flight run** — retrieval pins to the latest snapshot, and an INDEX task is *writing*
  a snapshot; pruning must not delete either out from under them.
- **No correctness coupling** — pruning only reclaims space; it must never change what a review retrieves.
- **Cheap + idempotent**, runnable on the existing singleton dispatcher, forward-compatible with the
  eventual overlay model.

## Decision

Add a periodic **index sweeper** to the dispatcher loop (alongside the task reaper, RFC-0001). Each cycle,
for every repo holding more than one snapshot, it computes a **keep-set** and prunes everything else from
both stores:

- **keep-set** = the **latest indexed commit** (`latest_indexed_commit`, what retrieval pins to) ∪ the
  `head_sha` of every **non-terminal** task for that repo (status not in
  `succeeded|failed|timed_out|cancelled`) — this protects a running review and an INDEX task mid-write.
- A **recency grace**: `code_chunks` rows indexed within the last 10 minutes are never pruned — a
  belt-and-suspenders for a just-finished index whose task hasn't flipped to terminal yet.
- **Safety floor**: an empty keep-set is a **no-op** in both `db::prune_code_chunks` and
  `neo4j::prune_graph` — the sweeper never wipes a live index even if the keep-set resolves empty.

Pruning is **storage GC only** (idempotent deletes, keep-set-guarded), so it stays correct even if more
than one replica ever runs the loop. Default cadence **600s** (storage isn't urgent), configurable via
`dispatcher.prune_interval_seconds`. New metrics: `lci_index_prune_total{outcome}` +
`lci_index_prune_{chunks,graph_nodes}_deleted_total`.

This is the **pre-overlay form** of RFC-0002's layer GC: the unit pruned today is a whole `(repo, commit)`
snapshot; once overlays land (Phase 1–2) the same sweeper generalizes to "keep base + live overlays".

## Consequences

- **Good:** index storage in pgvector + Neo4j is bounded to the latest snapshot + whatever in-flight runs
  pin; no per-commit accumulation; no change to retrieval behavior; reuses the proven reaper-loop shape.
- **Cost:** a small periodic scan (only repos with >1 snapshot do any delete work) and two new prune
  queries. The 10-minute grace means freshly-superseded snapshots linger briefly — acceptable for GC.
- **Limitation:** keying "in use" off non-terminal tasks' `head_sha` + the latest snapshot is a heuristic;
  the recency grace + empty-keep no-op are the safety nets. A future overlay model (RFC-0002 Phase 1–2)
  replaces the snapshot unit with explicit layers but keeps the same sweep.

## References

- [RFC-0002](../rfc/0002-incremental-layered-indexing.md) — incremental/layered indexing (this is its
  storage-bounding step, brought forward).
- [ADR-0050](0050-retrieval-pins-to-latest-indexed-snapshot.md) — reviews reuse the latest snapshot (why
  older snapshots are dead weight).
- [ADR-0025](0025-review-reuses-base-index.md) — reviews reuse the base index.
- RFC-0001 — the task reaper, whose dispatcher-loop pattern the sweeper mirrors.
