# ADR-0051: Per-model configuration blocks (primary / fallback / embeddings)

- **Status:** Proposed
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

Introduce a **per-model config block**. Each model the runner uses carries its own configuration; the
agent applies the config of the model it is actually using.

### Shape (ai-helm-values)

```yaml
agents:
  embeddings:
    baseUrl: …
    model: qwen3-embedding-8b
    config:                       # embeddings-specific knobs (small; grows as needed)
      requestTimeoutSecs: 60
  llm:
    baseUrl: …
    model: adorsys-reviewer
    config:                       # full per-model config
      contextWindow: 204800
      temperature: 0.2
      maxTokens: 8192
      maxTurns: 150
      maxBatchSize: 8
      maxFilesRead: 30
      maxSearches: 15
      maxBatches: 6
      maxDiffChars: 60000
      requestTimeoutSecs: 180
      maxRetries: 2
      circuitBreakerThreshold: 3
    fallback:
      model: adorsys-reviewer-pro
      config:                     # the fallback's OWN full config (own window, params, timeouts)
        contextWindow: 204800
        requestTimeoutSecs: 240
```

The chart maps these into nested `agent.json` blocks (`review.model.config`, `review.fallback.config`,
`embeddings.config`); the runner reads them into a `ModelConfig` struct.

### Which knobs follow which model (the one wrinkle)

Per the decision to make **everything per-model**, every knob lives in each model's `config`. But the
review loop is a *single sequence of turns* that mostly runs on the primary and may fail over to the
fallback for an individual turn — so two classes of knob behave differently at runtime:

- **Per-request knobs follow the active model.** `contextWindow` (the trim/wind-down threshold),
  `temperature` / `topP` / `maxTokens`, and `requestTimeoutSecs` / `maxRetries` are applied for the
  model issuing that turn's request — so a turn that fails over to the fallback uses the fallback's
  window and params.
- **Loop-cumulative knobs come from the primary.** `maxTurns`, the read budgets
  (`maxFilesRead`/`maxSearches`/`maxBatches`/`maxBatchSize`), `maxDiffChars`, and the
  `circuitBreakerThreshold` describe the *run*, which is one sequence — they are read from the
  **primary** model's config. The fallback may still carry these (uniform shape), but they are not used
  to redefine the run mid-flight (a failover must not, e.g., reset the turn ceiling).

This keeps the config uniform (a full block per model) while preserving sane loop semantics.

### Backward-compatible rollout (the `deny_unknown_fields` trap)

`ReviewFile` uses `#[serde(deny_unknown_fields)]`, and the agent-runner runs as a per-task Job pulling a
pinned image — so changing `agent.json`'s shape out from under a lagging runner image would fail every
Job (the config-rollout gotcha). To deploy safely in any order:

1. The new runner **dual-reads**: it accepts the new nested `model`/`fallback`/`config` blocks **and**
   the existing flat fields (`context_window`, `temperature`, `fallback_model`, …). Nested wins; flat is
   the fallback. So both the old and the new `agent.json` parse.
2. Ship the runner first (dual-read), then the chart (emits nested), then the values restructure.
3. A later cleanup PR removes the flat fields once nothing emits them.

## Consequences

- **Good:** each model is configured independently — the fallback gets its own window/timeouts (a slower
  `-pro` can have a longer timeout), embeddings gets its own block, and `contextWindow` matches each
  model's real window (no more early trims). Uniform, discoverable structure.
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
