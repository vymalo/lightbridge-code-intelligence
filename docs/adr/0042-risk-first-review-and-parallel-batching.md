# ADR-0042: Risk-first review strategy + parallel tool batching (Phase 1)

- **Status:** Accepted
- **Date:** 2026-06-23
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) reviews a PR by calling
retrieval/`read_file` tools one turn at a time until it `finish`es. In production it explores **too
sequentially**: it can burn 60+ turns and still produce a mediocre review. The bottleneck is not the
turn budget (`max_turns`) — it is that the agent reads the repo linearly and undirected, paying a full
model round-trip per tool call, and then emits low-signal findings.

Two structural facts shape the fix:

1. The model can already emit **parallel tool calls** in one assistant turn, but the dispatch loop runs
   them **serially** (`for call in &calls { dispatch(call).await }`). So even a batched turn pays
   sequential latency — the batching the model *can* do buys nothing today.
2. "How to review" (strategy, what to prioritise, what to publish) is operator-owned prompt territory
   (ADR-0037); the *mechanism* (what's parallelisable, what budgets exist) is code.

This is epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177). This ADR
covers **Phase 1** (strategy + speed); Phase 2 (explicit risk-map tool, confidence schema, finding
caps) is a separate ADR.

## Decision Drivers

- **Read smarter and faster, not more.** Fewer round-trips (real batching) and a risk-first order.
- **Every batch ties to a hypothesis** ("could this break authorization / an existing caller / a
  migration?"); general exploration is the signal to stop and write the review.
- **Don't regress** the #173 wind-down convergence or ADR-0040/0041 (prior-review context, coverage
  gate).
- **Operator-tunable budgets**, safe defaults, no required ai-helm change to keep working (ADR-0037
  fail-soft philosophy).

## Decision (Phase 1)

1. **Parallelise read-only tool dispatch within a turn.** Partition a turn's tool calls into:
   - *read-only* (`vector_semantic_search`, `graph_*`, `read_file`, `report_progress`) — executed
     **concurrently** (`join`), results echoed back in the model's original call order;
   - *write* (`add_review_comment`, `add_comment`) — applied **in original order** afterward (the
     control-plane buffer dedups by `(file,line)` last-write-wins, so order is semantically load-bearing);
   - *terminal* (`finish`, `abort`) — handled last, unchanged.
   This makes the model's batching actually fast without touching buffer/convergence semantics.

2. **Strategic budgets** on `ReviewConfig`, enforced in the loop (reusing the wind-down lever — drop the
   relevant tools / inject a "produce the review now" message when a budget is hit):
   - `max_batches`, `max_batch_size` (cap parallel read-only calls per turn),
   - `max_files_read`, `max_searches` (cumulative read budget).
   Hitting a read budget transitions the run toward `finish`, the same way wind-down does.

3. **Risk-first prompt** (ai-helm-values `reviewSystemPrompt`): the Phase 1–6 workflow (intent → risk
   map → hypothesis-driven parallel batches → candidates → filter → publish), the risk-area priority
   list, the "if the next batch is general exploration, stop" rule, and the output-quality rules (no
   gratuitous summary; no style/naming unless it points to a real risk; no generic "add tests"; every
   finding = severity + file/line + failure mode + impact + fix/test).

4. **Adapt the coverage gate (ADR-0041)** to key off **risk-map coverage** (every changed file is
   classified) rather than "every file was read" — risk-first deliberately does *not* read every file
   linearly, so the gate's notion of "engaged" widens to "accounted for in the plan".

Defaults (operator-tunable): `maxTurns 50, maxBatches 6, maxBatchSize 8, maxFilesRead 30, maxSearches
15`.

### Rollout (within Phase 1)

Phase 1 lands in slices so each is small and verifiable:

1. **Parallel read-only dispatch + `max_batch_size`** (this slice) — the latency lever; self-contained,
   no behaviour change to write/terminal handling.
2. **Cumulative read budgets** (`max_files_read` / `max_searches` / `max_batches`) — enforced by
   dropping the exhausted tool from the offered set and nudging toward `finish`, reusing the wind-down
   mechanism.
3. **Risk-first prompt** in ai-helm-values (the strategy lever).

The coverage-gate adaptation (key off risk-map coverage) is deferred to Phase 2, where the explicit
risk-map artifact exists to key off.

## Consequences

- **Good:** fewer round-trips → lower latency; directed investigation → higher signal; budgets bound
  cost; the model's existing parallel-call ability finally pays off.
- **Cost / risk:** concurrent tool execution must preserve write-buffer ordering and cancellation
  semantics (mitigated by only parallelising read-only calls). Budgets that are too tight could cut a
  review short — defaults are generous and operator-tunable, and the existing `Exhausted` backstop still
  posts buffered findings.
- **Phase 2** builds the explicit `record_risk_map` tool, the `confidence` field + `minConfidence`
  filter, and `maxCandidate/maxPublishedFindings` with a configurable `overCapPolicy`
  (`protect-blockers` never drops a confirmed P0/P1; `hard` is strict top-N).

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0037](0037-agent-acts-via-mediated-tools.md), [ADR-0040](0040-re-review-reads-prior-findings.md),
  [ADR-0041](0041-full-diff-coverage-gate.md); the #173 wind-down convergence.
