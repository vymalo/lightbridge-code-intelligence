# ADR-0051: Per-model configuration blocks (primary / fallback / embeddings)

- **Status:** Accepted — runner shipped ([#195](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/195)); the chart + values per-tier restructure shipped via [ADR-0062](0062-two-tier-review-fast-auto-deep-on-demand.md).
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning

## Context and Problem Statement

Model configuration today is one flat `config.model.*` block in ai-helm-values, mapped by the chart
into a flat `review` object in `agent.json` and read into a flat `ReviewConfig`. That block carries a
single `contextWindow`, `temperature`, `topP`, `maxTokens`, plus loop knobs (`maxTurns`, read budgets,
`maxDiffChars`) and resilience (timeout/retries/breaker). But the runner drives **three** models:

- the **primary** review model (`adorsys-reviewer`),
- the **fallback** review model (`adorsys-reviewer-pro`, used on per-turn failover — ADR-0039), today
  just a bare `fallbackModel` string with **no config of its own**, and
- the **embeddings** model (`qwen3-embedding-8b`), with no config block at all.

This bit us live: `contextWindow` was set once (128k, then 192k) for "the model", but MiniMax-M2's real
window is **204,800**, so the ADR-0045 budget trimmed tool output early (~96k on the 128k setting,
observed on run `99e28367`/`29360817`). And on failover the fallback silently inherits the primary's
window and generation params, which is wrong if they ever differ. There is no place to express
"this model has these settings."

## Decision

Give each **secondary** model the runner uses its own configuration block. The primary's existing flat
config *is* its per-model block, so it stays flat (this is also what keeps the rollout dual-read — see
below); the **fallback** and **embeddings** models, which had no config of their own, get a nested
`config` block. The agent applies the config of the model it is actually using.

> **As-built note.** An earlier draft of this ADR nested the *primary* too (`llm.config.*`). The runner
> that shipped (#195) deliberately kept the primary flat: nesting it would have broken the
> `deny_unknown_fields` dual-read (a lagging runner image couldn't parse the new shape), and the
> primary already had a complete flat block. So the per-model addition is scoped to the two models that
> lacked config — fallback and embeddings.

### Shape (ai-helm-values)

```yaml
agents:
  embeddings:
    baseUrl: …
    model: qwen3-embedding-8b
  llm:
    baseUrl: …
    model: adorsys-reviewer
config:
  embeddings:
    requestTimeoutSecs: 60        # NEW: embeddings-specific knobs (small; grows as needed)
  model:                          # the PRIMARY's config — stays flat (its existing block)
    contextWindow: 204800
    temperature: 0.2
    maxTokens: 8192
    maxTurns: 150
    maxDiffChars: 60000
    requestTimeoutSecs: 180
    maxRetries: 2
    fallback:                     # NEW: the fallback model with its OWN per-request config
      model: adorsys-reviewer-pro
      config:
        requestTimeoutSecs: 240   # a slower -pro won't time out on the primary's budget
        temperature: 0.2
```

The chart maps the primary's `config.model.*` into flat `review.*` (unchanged), and the new blocks into
nested `agent.json` blocks (`review.fallback{model,config}`, `embeddings.config`); the runner reads the
fallback/embeddings tuning into `FallbackConfig` / `EmbeddingsConfig`.

### Which knobs follow which model

The review loop is a *single sequence of turns* that mostly runs on the primary and may fail over to the
fallback for an individual turn, so knobs fall into two classes:

- **Per-request knobs follow the active model.** `temperature` / `topP` / `maxTokens` and
  `requestTimeoutSecs` / `maxRetries` are applied for the model issuing that turn's request — so a turn
  that fails over to the fallback runs under the fallback's generation params + timeout + retry budget.
  These are the fallback's `config`.
- **Run-level knobs come from the primary.** The context-budget **`contextWindow`** (the trim/wind-down
  threshold), `maxTurns`, the read budgets (`maxFilesRead`/`maxSearches`/`maxBatches`/`maxBatchSize`),
  `maxDiffChars`, and `circuitBreakerThreshold` describe the *run*, which is one sequence — so they are
  the **primary's**. In particular `contextWindow` is **not** a per-fallback knob: the trim happens
  *before* a turn is sent, when the loop can't yet know it will fail over, so it always uses the
  primary's window. A fallback with a smaller window is backstopped by the ADR-0045 tier-1
  overflow-finalize (never a lost finding), not a separate per-fallback trim. So the fallback's `config`
  carries only the per-request knobs above — not `contextWindow`, not the loop budgets.

This keeps the structure per-model while preserving sane loop semantics.

### Backward-compatible rollout (the `deny_unknown_fields` trap)

`ReviewFile` uses `#[serde(deny_unknown_fields)]`, and the agent-runner runs as a per-task Job pulling a
pinned image — so changing `agent.json`'s shape out from under a lagging runner image would fail every
Job (the config-rollout gotcha). To deploy safely in any order:

1. The primary's tuning stays **flat** on `review.*` (`context_window`, `temperature`, …) exactly as
   before — so an old `agent.json` still parses. The new runner additionally accepts the nested
   `review.fallback{model,config}` and `embeddings.config` blocks, **and** keeps reading the deprecated
   flat `review.fallback_model` string (nested `fallback` wins when both are present). So both the old
   and the new `agent.json` parse.
2. Ship the runner first (dual-read — done, #195), then the chart (emits `review.fallback` +
   `embeddings.config`), then the values restructure (sets the fallback's own timeout/params).
3. A later cleanup PR drops the deprecated `fallback_model` once nothing emits it.

## Consequences

- **Good:** each model is configured independently — the fallback gets its own generation params + a
  longer timeout (a slower `-pro` won't time out on the primary's budget), embeddings gets its own
  block, and the primary's `contextWindow` is set to the model's real window (no more early trims).
  Discoverable per-model structure.
- **Good:** the dual-read migration avoids the `deny_unknown_fields` deploy hazard — no lockstep needed.
- **Cost:** more config surface and a non-trivial refactor across three repos (runner config + loop,
  chart mapping, values). The primary/fallback `ModelConfig` is a shared struct; embeddings is a thin
  separate one. The flat fields linger until the cleanup PR.
- **Alternative considered:** keep loop knobs (`maxTurns`, budgets) at a shared `review` level and only
  make generation/window per-model. Rejected per the decision to make everything per-model; the
  loop-cumulative-from-primary rule above is the pragmatic reconciliation.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0045](0045-context-window-budget.md) — the context budget whose `contextWindow` this makes per-model.
- [ADR-0039](0039-agent-llm-resilience-and-observability.md) — the failover this gives its own config.
- Runs `99e28367` / `29360817` — early trims from a single under-set `contextWindow`.
