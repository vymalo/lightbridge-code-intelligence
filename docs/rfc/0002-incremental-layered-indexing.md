# RFC-0002: Incremental, layered indexing (base branch + per-PR overlays)

- **Status:** Proposed
- **Author(s):** Stephane Segning Lambou
- **Date:** 2026-06-20
- **Resulting ADRs:** [ADR-0050](../adr/0050-retrieval-pins-to-latest-indexed-snapshot.md) (Phase 0 —
  reviews reuse the latest indexed snapshot), [ADR-0052](../adr/0052-index-snapshot-pruning.md)
  (snapshot pruning / the index sweeper). Anticipated for the remaining phases: an ADR for the
  `ref`/overlay layer model + retrieval scoping, and one for the webhook-driven overlay lifecycle.

> **Update (2026-06-25):** **Phase 0 shipped** — reviews no longer re-index per PR; they reuse the
> latest indexed snapshot ([#183](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/183)
> index-on-push, [#188](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/188)
> commit-scope `repo_has_index`, [#194](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/194)/ADR-0050).
> This **reframes the remaining problem**: the *compute* waste is largely gone (warm repos don't
> re-index), so **storage accumulation is now the dominant issue** — and **pruning ships next**
> (ADR-0052, the *index sweeper*), ahead of the full overlay model below. See "Phasing".

## Summary

Indexing scopes everything by `(repository_id, commit_sha)`: full tree-sitter → pgvector embeddings
*and* full Graphify → Neo4j. This was wasteful twice over — it recomputed an index ~99% identical to
the default branch on every PR, **and** it accumulates a near-duplicate copy per commit in both stores
that nothing prunes. **Phase 0 (shipped) fixed the first half**: a review now reuses the latest indexed
snapshot instead of re-indexing (ADR-0050). The **second half remains** — every default-branch push
still writes a full new snapshot and the old ones are never reaped, so a busy repo's storage grows
without bound.

This RFC proposes **layered indexing**: index the **default branch once** (the *base layer*), and for
each PR index **only the changed files** as a thin **overlay layer** keyed by commit SHA on top of
the base. Retrieval reads `base ⊕ overlay` (the overlay shadows the base for files it changed).
Layers are managed by the **GitHub webhooks we already receive**: a PR's overlay is (re)built on
`opened`/`synchronize` and **deleted** on PR `closed`/merge and on branch deletion; the base is
refreshed on push to the default branch. This cuts per-PR indexing to the size of the diff and bounds
storage to "base + live PRs" instead of "every commit ever reviewed".

## Motivation

- **Cost.** Full indexing is the dominant cost of a task (the >15-min runs that motivated the 1-h Job
  deadline, #51). Re-embedding + re-graphing an entire repo for a 3-line PR is almost all waste.
- **Storage / correctness drift.** `code_chunks` (pgvector) and the Neo4j graph are written per
  `commit_sha` and never garbage-collected, so a busy repo accumulates dozens of full, stale copies.
  Nothing deletes the index for a merged or abandoned PR. The graph in particular grows unbounded.
- **Multiple branches, one repo.** Several open PRs on the same repo each index the full tree at their
  head — N copies that differ only by their diffs. The base is identical across all of them.

Expected outcome: a PR review indexes only its diff (fast, cheap), retrieval still sees whole-repo
context (base ⊕ overlay), and the datastores hold exactly one base per repo plus one small overlay
per *live* PR — with stale layers reaped as PRs close.

## Guide-level explanation

A **layer** is a set of indexed chunks + graph nodes/edges tagged with a *layer key*:

| Layer | Key | Built when | Deleted when |
|---|---|---|---|
| **base** | `(repo, ref=default_branch)` | first index of the repo; refreshed on push to the default branch | repo disconnected |
| **overlay** | `(repo, ref=pr/<number>, head_sha)` | PR `opened` / `synchronize` | PR `closed`/merged; branch deleted; sweeper for missed events |

A review for PR #N retrieves against **base ⊕ overlay(#N)**: for any file the PR changed, the overlay's
chunks/graph win; for every untouched file, the base provides context. So the agent still "sees" the
whole repository, but we only *computed* the diff.

Two flows, both driven by webhooks the control plane already ingests:

- **Push to default branch** → enqueue a *base reindex* task (full index of the default branch). This
  is the only full index, amortized across all PRs.
- **PR opened/synchronize** → enqueue an *overlay index* task that indexes **only the changed files**
  (`git diff base...head --name-only`) into the PR's layer, replacing any prior overlay for that PR.
- **PR closed/merged, branch deleted** → enqueue (or directly perform) an *overlay delete* that drops
  that PR's layer from pgvector + Neo4j.

## Reference-level explanation

### Layer key on stored rows

`code_chunks` and the Neo4j nodes/edges gain an explicit **layer** dimension instead of a bare
`commit_sha`:

- pgvector: add `ref text NOT NULL` (e.g. `@base` or `pr/42`) alongside `commit_sha`; index/scoped
  reads filter by `repository_id` + the layer set. The base layer uses a sentinel ref (`@base`).
- Neo4j: tag nodes/edges with `repo_id` + `ref` properties; the graph-query (find_symbol/get_callers)
  scopes to `{base, overlay}` and prefers overlay nodes for changed files.

### Retrieval: base ⊕ overlay

The control-plane retrieval API (`/search`, `/graph/query`) already scopes server-side per task
(trust boundary, ADR-0020). It gains the task's **layer set**: `[@base, pr/<n>]`. Semantic search
unions both layers and, when a file appears in the overlay, **excludes the base rows for that file**
(the overlay is authoritative for changed files) so a PR never sees pre-change chunks of a file it
edited. The graph query does the same shadowing by `source_file`.

### Indexing only the changed files (overlay)

The agent runner already computes the PR diff (`clone::pr_diff`, #54). The overlay task:
1. clones at head (shallow), 2. tree-sitter-chunks + embeds **only** the changed files, 3. runs
Graphify over the changed files (or the whole tree but submits only changed-file nodes), 4. submits
them tagged with the PR's layer key, replacing the prior overlay (delete-then-insert per PR ref).

A **dedup short-circuit**: if the PR's `head_sha` already has an overlay, skip re-indexing (a
re-delivered webhook or a no-op sync). The base reindex similarly skips if the default-branch SHA is
unchanged.

### Lifecycle (webhook-driven) + a safety sweeper

The webhook handler maps events to queue tasks:
- `pull_request: closed` (merged or not) → delete `pr/<number>` overlay.
- `delete` (branch) / `pull_request` head ref gone → delete the matching overlay.
- Because webhooks can be missed (the same reason the dispatcher has a reaper, RFC-0001), a periodic
  **index sweeper** reconciles: drop overlays whose PR is closed or whose branch no longer exists
  (queried via the GitHub API), and drop layers for disconnected repos. This is the storage analogue
  of the task reaper.

### Phasing (each step independently shippable)

- **Phase 0 (dedup) — ✅ SHIPPED.** Index the default branch on push (#183); skip per-PR re-indexing and
  pin both the skip-check and retrieval to the latest indexed snapshot (#188, #194 / ADR-0050). No
  schema change. Cut the compute waste immediately — the original motivation's biggest line item.
- **Snapshot pruning (the *index sweeper*) — ✅ SHIPPED (ADR-0052).** Brought forward ahead of the
  overlay model because, post-Phase 0, storage is the dominant remaining problem. A periodic dispatcher
  sweep (modeled on the task reaper) keeps only the in-use snapshots per repo — the latest indexed
  commit (what retrieval pins to) plus any commit an in-flight task pins, with a recency grace — and
  reaps the rest from pgvector + Neo4j. This is the pre-overlay form of the layer GC: today the unit is
  a whole `(repo, commit)` snapshot; once overlays land it generalizes to "keep base + live overlays".
- **Phase 1 (layer key + lifecycle delete):** add the `ref` layer dimension; delete a PR's layer on
  close/branch-delete via webhooks. (The sweeper above already bounds storage; this makes deletion
  prompt + per-PR.)
- **Phase 2 (overlay indexing):** index only changed files for a PR; retrieval reads base ⊕ overlay.
  The remaining *compute* win for multi-branch correctness (less urgent now Phase 0 removed per-PR reindex).
- **Phase 3 (webhook-driven delete + GitHub-API reconciliation):** prompt overlay delete on PR
  close/branch-delete + a reconciliation pass for missed events (the sweeper generalizes to this).

## Drawbacks

- More moving parts in retrieval (layer union + per-file shadowing) and a schema migration.
- A PR overlay can go stale vs. a fast-moving base (the base advanced after the overlay was built);
  acceptable — the overlay is rebuilt on the next `synchronize`, and base drift only affects unchanged
  files' context, not the changed lines under review.
- Graph overlays are trickier than chunk overlays (cross-file edges may reference base nodes); Phase 2
  must define how an overlay edge to an unchanged symbol resolves (likely: resolve against the base
  layer by symbol id).
- **Deleted files** need an explicit *tombstone* in the overlay. The shadow rule ("overlay wins for
  files it touches") only fires when a file *appears* in the overlay — but a file the PR **deletes**
  produces no overlay rows, so retrieval would fall back to the base and still surface the removed
  code. The overlay must record the PR's deleted paths and retrieval must exclude base rows for those
  paths, not just for paths that have overlay rows. Enumerate deletions with **`git diff --no-renames
  --diff-filter=D`** — `--no-renames` decomposes a rename into delete-old + add-new, so the *old* path
  of a renamed file is tombstoned (a plain `--diff-filter=D` with rename detection on would miss it,
  leaving the pre-rename file retrievable from the base).
- **Reverse graph edges (base → overlay).** An unchanged base file may call a symbol the PR *changed*;
  the base edge still physically points at the now-shadowed base node, so a normal traversal
  `(caller)-[:REL]->(target)` never reaches the overlay node. Resolving this purely at query time (join
  on symbol id instead of traversing the relationship) sacrifices Neo4j's index-free adjacency and
  turns traversals into pointer-chasing joins. Phase 2 should therefore **rewrite the boundary edges
  at overlay-ingestion time** — when an overlay node shadows a base symbol, re-point (virtualize) the
  incoming base edges to the overlay node within the query scope — preserving traversal performance,
  rather than relying on a query-time symbol-id lookup. Either way overlay-precedence by symbol id is
  the resolution rule; the trade-off is *when* it's applied.

## Alternatives considered

- **Keep full per-commit indexing + just add GC (TTL on old commits).** Bounds storage but keeps the
  compute waste; rejected as half a fix.
- **Cache by content hash per file (global), no layers.** More general dedup, but complicates
  per-repo scoping and the trust boundary; revisit if cross-repo dedup ever matters.
- **No base; index each PR fully but dedup identical commits.** Misses the "99% same as base" win.

## Unresolved questions

- Exact layer-key scheme (`@base` sentinel vs. storing the default-branch name) and the migration for
  existing `code_chunks` / Neo4j data.
- Graph overlay edge resolution across layers (Phase 2 detail above).
- Whether the base reindex is one task or chunked per top-level path for very large monorepos.
- Whether the sweeper lives in the dispatcher loop (like the task reaper) or a dedicated `scheduler`
  role (RFC-0001 Phase 2).
