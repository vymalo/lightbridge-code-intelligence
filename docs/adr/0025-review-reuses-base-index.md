# ADR-0025: Review reuses the base index instead of re-indexing every run

- **Status:** Accepted
- **Date:** 2026-06-21

## Context and Problem Statement

Every agent Job ran the **same pipeline** in `agent-runner`'s `run()`: clone → semantic index
(pgvector) → structural index (Neo4j) → then review. The indexing steps were unconditional, so a
**review job re-indexed the entire repository from scratch before reviewing** (re-embedding every file
and rebuilding the whole graph at the PR head commit). The only thing an `index` job did differently
was skip the final review step.

Symptom (observed in prod): a PR review takes roughly as long as a full repo index, every time — and
it re-embeds unchanged files on each review. See [jobs-and-lifecycle.md](../jobs-and-lifecycle.md).

## Decision Drivers

- A review's job is to evaluate the **diff**; the changed code is already handed to the agent in its
  prompt. The index exists to let the agent **search related context** via the MCP tools, where the
  base (default-branch) index is almost entirely sufficient.
- Re-embedding a whole repo per review is the dominant, mostly-wasted cost.
- Must stay correct for a cold repo (no base index yet) and degrade safely on errors.

## Decision Outcome

**A review reuses the existing base index; it does not re-index.** The control plane reports
`repo_indexed` in the runner's task context (`db::repo_has_index` — does the repo have any
`code_chunks`?). The runner indexes only when:

- the task is an **`index`** task (the base index for an approved repo), **or**
- the repo is **cold** (`repo_indexed == false`) — a one-time index so the first review isn't blind.

A review on an already-indexed repo skips both the semantic and structural index and goes straight to
the review, searching the base index via the MCP tools. `repo_has_index` failing (DB hiccup) is
treated as "not indexed" → the runner indexes — i.e. it fails **safe**, back to the old behavior.

### Consequences

- Good: a review now costs ≈ clone + review, not clone + full re-index + review — the main win the
  symptom called for. Reviews stop re-embedding unchanged files.
- Neutral: the base index reflects the **default branch**, not the PR head. The agent still sees the
  PR's changed code (it's in the prompt + diff); only *search over* brand-new symbols in the PR is
  limited until the next base re-index. Acceptable for review context.
- Bad / follow-up: a base index can go **stale** as the default branch moves. A periodic / push-driven
  re-index of the default branch (or **incremental diff-only indexing** for reviews) is the natural
  next step — tracked as a follow-up, not in this change.

### References

- Pipeline: `services/agent-runner/src/main.rs` (`run()`); `db::repo_has_index`,
  `internal::get_context` (`repo_indexed`). Lifecycle: [jobs-and-lifecycle.md](../jobs-and-lifecycle.md).
- Builds on the indexing baseline (ADR-0019) and the review agent (ADR-0021).
