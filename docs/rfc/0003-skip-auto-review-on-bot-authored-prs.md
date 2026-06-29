# RFC-0003: Skip the automatic review on bot-authored PRs

- **Status:** Accepted
- **Author(s):** Stephane Segning Lambou
- **Date:** 2026-06-29
- **Resulting ADRs:** [ADR-0067](../adr/0067-skip-auto-review-on-bot-authored-prs.md)

## Summary

When a pull request is **opened by a bot** (Dependabot, Renovate, a CI bot, another GitHub App, or
ourselves), the control plane should **not** launch the automatic fast review. Detection keys off the
PR author's GitHub account type (`pull_request.user.type == "Bot"`, plus the `[bot]` login suffix as a
defensive backstop). The behaviour is gated by a config knob, defaults to "skip", and **only**
suppresses the automatic on-`opened` review — a human can still trigger a deep review on a bot PR with
an `@mention`.

## Motivation

Today every `pull_request` `opened` event on an approved repo creates a **fast-tier** review task
([`handle_pull_request`](../../services/control-plane/src/http/webhook.rs), the `"opened"` arm,
[ADR-0062](../adr/0062-two-tier-review-fast-auto-deep-on-demand.md)). That fires unconditionally,
including for PRs that no human authored. Concretely this is a problem because:

- **Cost and noise on dependency bumps.** Dependabot/Renovate open a steady stream of mechanical
  version-bump PRs. Auto-reviewing each one spends an LLM turn budget and posts review comments that
  add little signal to a lockfile diff — exactly the low-value noise the fast tier is meant to avoid.
- **Bot-on-bot loops.** When another automated actor opens a PR, our review fires automatically, which
  can interleave with *their* automation. Keeping bots from auto-triggering each other is good
  hygiene; it removes a whole class of feedback-loop surprises before they happen.
- **Self-authored PRs.** If our own App (or a sibling automation) ever opens a PR, we don't want to
  auto-review our own mechanical change.

The cut is deliberately narrow: the *automatic* review is the thing that's unwanted on bot PRs. The
explicit `@mention` deep review is a human asking on purpose, and must keep working — a maintainer who
*does* want eyes on a suspicious dependency bump can still get the full repo-aware review.

## Guide-level explanation

A new review knob controls this:

```jsonc
// control-plane.json (mounted from the Helm ConfigMap)
{
  "review": {
    "skipBotAuthoredPrs": true   // default: true
  }
}
```

Behaviour, by trigger:

| Trigger | Author is a human | Author is a bot |
|---|---|---|
| `pull_request` `opened` (auto **fast** review) | review runs | **skipped** (logged) |
| `@mention` comment (**deep** review) | review runs | review runs |
| `push` to default branch (re-index) | unaffected | unaffected |

"Author is a bot" means the PR's `user.type` is `Bot` (GitHub reports this for App-backed accounts
like `dependabot[bot]` and `renovate[bot]`), with a secondary check that the login ends in `[bot]`.

When the auto review is skipped, the webhook still does everything else it normally does — the
delivery is persisted, the repo is upserted — it simply does not create the review task, and emits a
log line in the same spirit as the existing approval-gate skip:

```
pull_request opened by bot 'dependabot[bot]'; skipping automatic review (skip_bot_authored_prs=true)
```

Setting `skipBotAuthoredPrs: false` restores today's behaviour (auto-review everything). The default
is `true` because that is the desired posture for this repo and the common case across installations.

## Reference-level explanation

### Detection

The `pull_request` `opened` payload carries the author under `pull_request.user`. Add a small helper
alongside the existing payload parsing in
[`services/control-plane/src/http/webhook.rs`](../../services/control-plane/src/http/webhook.rs):

```rust
/// True when a PR was opened by a bot account. GitHub sets `user.type == "Bot"` for App-backed
/// authors (e.g. `dependabot[bot]`, `renovate[bot]`); the `[bot]` login suffix is a defensive
/// backstop in case `type` is ever absent/unexpected in a payload.
fn pr_author_is_bot(pull_request: &serde_json::Value) -> bool {
    let user = &pull_request["user"];
    let is_bot_type = user["type"].as_str() == Some("Bot");
    let has_bot_suffix = user["login"]
        .as_str()
        .is_some_and(|login| login.ends_with("[bot]"));
    is_bot_type || has_bot_suffix
}
```

This reads only fields already present on the `opened` payload — no extra GitHub API call.

### Gate placement

In `handle_pull_request`, the check sits inside the `"opened"` arm, **after** the approval gate
(`approved_or_skip`) and **before** `create_review_task`. The approval gate is the more fundamental
"should anything run at all" check, so it stays first; the bot check is a narrower "should the
*automatic* review run" check layered on top:

```rust
"opened" => {
    let Some(installation_id) = installation_id_opt else { return; };
    if !approved_or_skip(pool, repository_id, delivery_id, pr_number).await {
        return;
    }
    let pr = &payload["pull_request"];
    // RFC-0003: bot-authored PRs don't get the automatic fast review. A human can still
    // request the deep review via @mention (that path is untouched).
    if state.review.skip_bot_authored_prs() && pr_author_is_bot(pr) {
        let login = pr["user"]["login"].as_str().unwrap_or("<unknown>");
        tracing::info!(
            delivery_id, pr = pr_number, author = login,
            "PR opened by bot; skipping automatic review (skip_bot_authored_prs)"
        );
        crate::http::metrics::review_skipped_bot_author(); // new counter (see Observability)
        return;
    }
    // ... unchanged: build NewTask { tier: "fast", .. } and create_review_task(..)
}
```

The `@mention` path (`handle_issue_comment`) is **not** touched: an explicit human command still
always lands a deep-tier task, even on a bot PR.

### Config

Extend `ReviewSection` in
[`services/control-plane/src/config.rs`](../../services/control-plane/src/config.rs), matching the
existing `reactions: Option<bool>` + accessor idiom:

```rust
pub struct ReviewSection {
    pub reactions: Option<bool>,
    pub label_reviewed: Option<String>,
    pub label_findings: Option<String>,
    pub label_error: Option<String>,
    /// Skip the automatic on-`opened` review when the PR author is a bot. The @mention deep
    /// review is unaffected. Defaults to enabled (skip) when unset (RFC-0003).
    pub skip_bot_authored_prs: Option<bool>,
}

impl ReviewSection {
    pub fn skip_bot_authored_prs(&self) -> bool {
        self.skip_bot_authored_prs.unwrap_or(true)
    }
}
```

The accessor is surfaced on the same handle the webhook already uses for `state.review.reactions_enabled()`,
so no new plumbing is needed.

### Observability

Add a `review_skipped_bot_author` counter next to the existing webhook/task metrics in
[`services/control-plane/src/http/metrics.rs`](../../services/control-plane/src/http/metrics.rs), so a
dashboard panel can show how many auto-reviews were suppressed for bot authorship (distinct from the
approval-gate skip). The existing `task_created` counter already covers the "review did run" side.

### Edge cases

- **`user.type` missing/garbled.** The `[bot]` login suffix backstop covers it; if both are absent the
  PR is treated as human-authored (fail *open* — we'd rather over-review than silently drop a real
  PR). This matches the conservative direction: never silently skip a human's PR.
- **Forks / first-time contributors.** Unaffected — those are human `User` accounts; only `Bot`
  accounts are gated.
- **Re-open / synchronize.** Already no-ops today (re-review is `@mention`-only), so nothing changes.
- **Disabling the feature.** `skipBotAuthoredPrs: false` reverts to current behaviour with no code
  path differences beyond the single guard.

### Migration / rollout

Pure additive config; no DB migration. Ships dark-compatible: an installation with no
`review.skipBotAuthoredPrs` key gets the new default (`true`). If we want to stage it, land the code
with the default `false` first, flip to `true` in `ai-helm-values` after one dogfood cycle — but the
recommendation is to default `true` from the start, since that is the intended posture.

## Drawbacks

- **Dependency bumps skip the deterministic SAST pass too.** The fast tier runs opengrep SAST
  ([ADR-0061](../adr/0061-sast-deterministic-finding-source.md)) before the LLM. Skipping the auto
  review on bot PRs means a malicious or vulnerable dependency bump no longer gets that automatic SAST
  scan. Mitigations: a maintainer can `@mention` for the deep review (which also runs SAST), and most
  bumps are lockfile/manifest-only where SAST has little to say. Still, this is a real reduction in
  automatic coverage on exactly the PRs (third-party code changes) where supply-chain risk lives — see
  Unresolved questions for a SAST-only middle ground.
- **Coarse "all bots" treatment.** Some teams may want a *specific* bot (e.g. an internal codegen bot)
  to still be auto-reviewed. The v1 knob is a single boolean; per-bot granularity is deferred (see
  Alternatives / Unresolved questions).
- **One more config knob.** Marginal added surface in `ReviewSection`, mitigated by reusing the exact
  `Option<bool>` + accessor pattern already there for `reactions`.

## Alternatives

- **Do nothing (status quo).** Keep auto-reviewing bot PRs. Rejected: it's the noise/cost/loop problem
  the motivation describes, and the user explicitly wants bots to not trigger the review.
- **Allowlist of human authors (inverse).** Only auto-review PRs from a configured set of human logins.
  Rejected: high maintenance, breaks for new contributors, and inverts the natural default.
- **Label/title heuristics** (e.g. skip PRs labelled `dependencies` or titled `chore(deps): …`).
  Rejected: fragile and bot-specific; `user.type == "Bot"` is the canonical, bot-agnostic signal
  GitHub already provides.
- **Author-association gate** (skip non-`MEMBER`/`OWNER`). Rejected: conflates "outside human
  contributor" with "bot" — we *do* want to review outside human PRs.
- **Per-bot allow/deny list instead of a boolean.** More flexible but more config to reason about;
  start with the boolean and add the list later only if a need shows up (Unresolved questions).

## Unresolved questions

- **SAST-only middle ground.** Should a bot PR optionally still get the deterministic SAST pass
  (cheap, no LLM) while skipping the LLM review — i.e. a third state `sast_only` rather than a boolean?
  This would recover supply-chain coverage on dependency bumps at near-zero cost. Likely a fast
  follow-up rather than v1.
- **Per-bot allowlist.** If some installations want specific bots auto-reviewed (or specific bots
  *always* skipped regardless of the global flag), extend the knob to `skipBotAuthoredPrs: ["renovate[bot]", …]`
  or a `{ default, allow, deny }` shape. Deferred until there's a concrete need.
- **Should the default be `true` or `false` on first ship?** Proposed `true` (skip) to match the
  intended posture; open to landing `false` and flipping in `ai-helm-values` after one dogfood cycle.
- **Out of scope:** changing the `@mention` deep-review path, the `push`/re-index path, the approval
  gate, or anything about how non-bot PRs are reviewed.
