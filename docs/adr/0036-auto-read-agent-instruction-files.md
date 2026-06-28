# ADR-0036: Auto-read conventional agent instruction files (AGENTS.md, CLAUDE.md, …) as review context

- **Status:** Accepted
- **Date:** 2026-06-22

## Context and Problem Statement

The ecosystem has converged on repo-native agent instruction files — `AGENTS.md` (the vendor-neutral
[agents.md](https://agents.md) convention), `CLAUDE.md`, `.github/copilot-instructions.md`,
`.cursorrules` — that capture how a codebase wants automated agents to behave. A review that ignores
them re-derives conventions the repo already stated. We want the review agent to **read these by default
at the start of a review, if present**, in a defined **ranking**, so it respects the repo's own rules
with zero per-repo setup.

[ADR-0030](0030-repo-review-config.md) already lets a repo declare **explicit** review config in
`.lightbridge-code-review.jsonc`. This ADR adds **convention-based, zero-config discovery** of the
above files. How do we layer the two, in what precedence, and how do we stay safe — these files are
**untrusted repo content** that will steer a reviewer that flags security issues?

## Decision Drivers

- **Respect repo conventions with zero config** — the common case shouldn't need our config file.
- **Deterministic precedence** when several files exist (the user's "ranking").
- **Safety:** untrusted text must not override our review mission, tools, credentials, or suppress
  findings (prompt-injection) — same posture as [ADR-0030](0030-repo-review-config.md)/
  [ADR-0031](0031-review-skills-commands.md).
- **Bounded:** these files can be large; cap what we ingest.

## Considered Options

- **A. Ranked auto-discovery** of a known set of convention files, read those present in rank order, fold
  into the prompt as **labelled, untrusted** context; our explicit config ([ADR-0030](0030-repo-review-config.md))
  still wins; default on, toggle/reorder via config.
- **B. Explicit config only** ([ADR-0030](0030-repo-review-config.md)); ignore convention files.
- **C. Ingest all docs** (README, CONTRIBUTING, `docs/**`) as context.

## Decision Outcome

Chosen option: **A.** At review start the runner (which already clones the repo and computes the diff,
`agent-runner/src/clone.rs`) discovers the following, **highest precedence first**:

1. **`.lightbridge-code-review.jsonc`** — our explicit config ([ADR-0030](0030-repo-review-config.md)).
2. **`AGENTS.md`** — vendor-neutral convention.
3. **`CLAUDE.md`**
4. **`.github/copilot-instructions.md`**
5. **`.cursorrules`** / **`.cursor/rules/*`**

All present files are read, in rank order, and folded into the prompt **labelled with their source and
precedence**. On conflict the agent is told to favour the higher-ranked source. Crucially, **our review
mission, the output contract ([ADR-0026](0026-native-review-agent.md)/[ADR-0032](0032-review-finding-priority-and-category.md)),
the tool set, and the diff validation/write-back ([ADR-0022](0022-review-writeback-control-plane.md))
remain authoritative and cannot be overridden** by any ingested file. Total ingested size is **capped
and truncated** — default **~32 KiB total** across all files (per-file truncation, highest-ranked
first), **configurable** via the explicit config — so an oversized or hostile file can't exhaust the
context window or inflate cost. The behaviour is **on by default**; the explicit config can disable it,
reorder the ranking, or add/remove paths.

> Refinement (not v1): `AGENTS.md` supports **per-directory nesting** (closest file wins). A later
> increment can also read the `AGENTS.md` nearest each changed directory, not just the repo root.

### Trust model

A discovered file is **untrusted prompt text**. It can steer the agent's *reasoning and emphasis* only —
it **cannot** run commands, change credentials, alter the tool set, change our mission, or suppress
findings. The classic injection ("*ignore all security issues*") is mitigated by: (a) mission/contract
precedence over ingested text, (b) ADR-0022 re-validation of every finding against the diff before
posting, (c) the size cap, and (d) labelling the content as untrusted repo input in the prompt.

### Consequences

- Good: reviews respect each repo's stated conventions with **no setup**; aligned with the AGENTS.md
  ecosystem standard; layers cleanly under the explicit config.
- Bad: a real prompt-injection surface (mitigated as above); a ranking + size/precedence policy to
  maintain; more prompt tokens per review.
- Neutral: implemented in the runner at clone time and merged in `build_prompt`; complements, doesn't
  replace, [ADR-0030](0030-repo-review-config.md)/[ADR-0031](0031-review-skills-commands.md). Within the
  [ADR-0029](0029-focused-review-not-generic-runner.md) boundary (context, not execution).

## Pros and Cons of the Options

### A. Ranked auto-discovery (chosen)
- Good: zero-config, ecosystem-aligned, deterministic, layered under explicit config.
- Bad: injection surface + precedence/size rules to own.

### B. Explicit config only
- Good: nothing untrusted ingested unless opted in; simplest trust story.
- Bad: setup friction; ignores files the repo already maintains for other agents — the user's exact ask.

### C. Ingest all docs
- Good: maximal context.
- Bad: noisy, unbounded, not agent-targeted; dilutes the signal and worsens injection surface.

## More Information

- Layers under [ADR-0030](0030-repo-review-config.md) (explicit config) and feeds the same agent loop as
  [ADR-0026](0026-native-review-agent.md); output still validated/posted per
  [ADR-0022](0022-review-writeback-control-plane.md); within [ADR-0029](0029-focused-review-not-generic-runner.md).
- Discovery + read in `agent-runner/src/clone.rs`; merge in `agent-runner/src/review/mod.rs`
  (`build_prompt`). Convention reference: <https://agents.md>.
