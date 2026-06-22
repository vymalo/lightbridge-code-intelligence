# ADR-0031: Custom review skills/commands via the repo config

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

[ADR-0030](0030-repo-review-config.md) lets a repo declare **static** review context. We also want a
repo to define **named, invokable review behaviors** — "more commands or skills" — specific to that
codebase, building on the **existing `@mention` command path** (the webhook already dispatches tasks
from PR/issue comments). How do we offer that *without* executing arbitrary code
([ADR-0029](0029-focused-review-not-generic-runner.md))?

## Decision Drivers

- **Extend what the reviewer can do per-repo** (mission) — as **prompts/instructions, not binaries**
  (ADR-0029).
- **Reuse the existing trigger:** the `@mention` webhook command path already exists.
- **Bounded + data-only:** keep security and maintenance low.

## Decision Outcome

Add a **`skills`** (a.k.a. `commands`) block to `.lightbridge-code-review.jsonc` (ADR-0030). Each skill
is a **named prompt/instruction set** (plus optional focus/retrieval hints) that the native review agent
([ADR-0026](0026-native-review-agent.md)) runs — **data, not code.** Sketch:

```jsonc
{
  "skills": {
    "security-pass": {
      "instructions": "Audit auth and input validation against OWASP Top 10; flag missing authz checks.",
      "auto": false
    },
    "api-versioning": {
      "instructions": "Enforce our semver + deprecation policy.",
      "focus": ["api/**"],
      "auto": true            // runs as part of every review
    }
  }
}
```

Two invocation modes:

- **Automatic:** a skill marked `auto: true` is folded into every review (e.g. "always check our
  API-versioning rules").
- **On demand:** `@lightbridge security-pass` in a PR/issue comment → the **existing webhook command
  path** dispatches a task that runs that skill.

A skill is dispatched through the **same agent loop + control tools** (`submit_findings`, ADR-0026) and
the **same control-plane validation + write-back** ([ADR-0022](0022-review-writeback-control-plane.md))
as a normal review. No new execution surface, no new credentials.

### Trust model

Same as [ADR-0030](0030-repo-review-config.md): a repo-provided skill is **untrusted prompt text**. It
can only steer the agent's own reasoning and output, which the control plane still re-validates against
the PR diff before posting (ADR-0022). A skill **cannot** run commands or exfiltrate — the agent's tools
are control-plane-proxied ([ADR-0020](0020-mcp-servers-via-control-plane.md)). Residual risk is
prompt-injection-style steering of the agent; mitigated by ADR-0022 validation and by capping/labelling
skill-produced output.

### Consequences

- **Good:** per-repo reviewer customization (the user's "commands/skills") **within** the focused
  mission; reuses the `@mention` path and the existing agent loop; data-only — no new execution surface.
- **Bad:** prompt-injection-style risk from repo-authored skill text (mitigated by ADR-0022 validation +
  output capping); `@mention` name disambiguation/UX; a skill registry + precedence to define.
- **Neutral:** depends on [ADR-0030](0030-repo-review-config.md) (same file) +
  [ADR-0026](0026-native-review-agent.md) (the loop) + the webhook command path; org-level skills are a
  future option.

## Pros and Cons of the Options

### Skills as named prompts in the repo config (chosen)

- Good: mission-aligned customization; reuses existing trigger + loop; data-only/safe.
- Bad: prompt-injection nuance; naming/UX + registry to define.

### No skills (only static context, ADR-0030)

- Good: simplest; least to maintain.
- Bad: no per-repo invokable behaviors; users can't name and trigger repo-specific review passes.

## More Information

- Depends on [ADR-0030](0030-repo-review-config.md) (the config file) and
  [ADR-0026](0026-native-review-agent.md) (the agent loop + control tools); output validated/posted per
  [ADR-0022](0022-review-writeback-control-plane.md); tools proxied per
  [ADR-0020](0020-mcp-servers-via-control-plane.md). Within the boundary of
  [ADR-0029](0029-focused-review-not-generic-runner.md).
- Expect the follow-up RFC (with ADR-0030) to fix the skill schema, the `@mention` grammar, and the
  registry/precedence.
