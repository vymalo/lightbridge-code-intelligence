# ADR-0067: Skip the automatic review on bot-authored PRs

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** @stephane-segning

## Context and Problem Statement

Every `pull_request` `opened` event on an approved repo unconditionally creates a **fast-tier** review
task ([ADR-0062](0062-two-tier-review-fast-auto-deep-on-demand.md)). The `"opened"` arm of
`handle_pull_request` (`services/control-plane/src/http/webhook.rs`) builds `NewTask { tier: "fast", .. }`
and calls `create_review_task` with no regard for **who** opened the PR.

A large share of opened PRs are not authored by humans: Dependabot, Renovate, CI bots, sibling GitHub
Apps — and potentially ourselves. Auto-reviewing those:

- **spends LLM turn budget** on mechanical, low-signal diffs (lockfile/manifest bumps), exactly the
  noise the fast tier exists to keep cheap;
- **posts review comments that add little** to a version-bump diff;
- **risks bot-on-bot loops** — our automation reacting to another actor's automation.

The fast tier already made *every PR* cheap; this ADR makes *bot PRs* free by not running it at all,
without touching the deliberate, human-requested deep review.

This decision was socialized as [RFC-0003](../rfc/0003-skip-auto-review-on-bot-authored-prs.md).

## Decision

**Do not create the automatic fast-tier review task when the PR author is a bot.** The cut is
deliberately narrow — only the *automatic on-`opened`* review is suppressed.

### Detection
A PR is bot-authored when, on the `opened` payload, `pull_request.user.type == "Bot"` (GitHub reports
this for App-backed accounts such as `dependabot[bot]` / `renovate[bot]`), **or** — a defensive
backstop — `pull_request.user.login` ends in `[bot]`. Both fields are already on the payload, so there
is **no extra GitHub API call**. When neither signal is present the PR is treated as **human-authored
(fail open)** — we would rather over-review than silently drop a real contributor's PR.

### Gate placement
The check sits in the `"opened"` arm of `handle_pull_request`, **after** `approved_or_skip` (the
fundamental "should anything run at all" gate stays first) and **before** `create_review_task`. On a
skip the webhook still persists the delivery and upserts the repo; it simply creates no task, emits an
info log mirroring the approval-gate skip, and increments a `review_skipped_bot_author` counter
(`services/control-plane/src/http/metrics.rs`), distinct from the approval skip.

### What is untouched
- **The `@mention` deep path** (`handle_issue_comment`) always creates its deep-tier task, even on a
  bot PR — a human can still request a full repo-aware review on a Dependabot bump.
- The `push`/re-index path and the approval gate are unchanged.

### Config
A new `review.skip_bot_authored_prs` knob (`ReviewSection` in `services/control-plane/src/config.rs`),
`Option<bool>` with a `skip_bot_authored_prs(&self) -> bool` accessor **defaulting to `true`** —
mirroring the existing `reactions` / `reactions_enabled()` idiom. `false` restores today's behaviour
(auto-review everything) with no other code-path difference. The JSON key is **snake_case**
(`skip_bot_authored_prs`, matching `reactions` / `label_reviewed`): `ReviewSection` derives `Deserialize`
with `deny_unknown_fields` and **no `rename_all`**, so a camelCase key would be rejected at config parse
rather than silently bind.

## Consequences

- **Good:** bot PRs cost nothing automatically; the noise/cost/loop problem is removed at the source;
  the change is one guard + one config knob, reusing existing idioms; reverting is a single flag flip.
- **Reduced automatic coverage on dependency bumps — by design.** Skipping the auto review also skips
  the deterministic opengrep SAST pass ([ADR-0061](0061-sast-deterministic-finding-source.md)) on
  exactly the PRs (third-party code) where supply-chain risk concentrates. Mitigation: a maintainer can
  `@mention` for the deep review (which runs SAST); a **SAST-only middle ground** for bot PRs is
  recorded as a deferred follow-up (RFC-0003 *Unresolved questions*), not built here.
- **Coarse "all bots" treatment.** v1 is a single boolean; per-bot granularity (allow/deny list) is
  deferred until there is a concrete need.
- **Fail-open mis-detection risk:** an unusual payload lacking both signals is treated as human and gets
  reviewed — the safe direction; covered by unit tests on `pr_author_is_bot`.

## Alternatives considered

- **Do nothing (status quo).** Keep auto-reviewing bot PRs. Rejected — it is the noise/cost/loop problem
  this ADR exists to remove.
- **SAST-only on bot PRs** (run opengrep, skip the LLM turn). Attractive — recovers supply-chain
  coverage at near-zero cost — but it is a third behaviour, not yet decided; deferred to a follow-up.
- **Per-bot allow/deny list instead of a boolean.** More flexible, more config to reason about; start
  with the boolean, extend only if a need appears.
- **Label/title heuristics** (skip `dependencies`-labelled or `chore(deps):` PRs). Rejected — fragile and
  bot-specific; `user.type == "Bot"` is the canonical, bot-agnostic signal GitHub already provides.
- **Author-association gate** (skip non-`MEMBER`/`OWNER`). Rejected — conflates outside *human*
  contributors with bots; we do want to review outside human PRs.

## References

- [RFC-0003](../rfc/0003-skip-auto-review-on-bot-authored-prs.md) — the proposal this ADR records.
- [ADR-0062](0062-two-tier-review-fast-auto-deep-on-demand.md) — the two-tier model; this gates the fast auto tier only.
- [ADR-0061](0061-sast-deterministic-finding-source.md) — the SAST pass that bot PRs forgo automatically.
- Implementation ticket: #258.
