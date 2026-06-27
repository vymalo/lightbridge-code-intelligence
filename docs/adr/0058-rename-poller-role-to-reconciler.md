# ADR-0058: Rename the `poller` role to `reconciler`

- **Status:** Accepted
- **Date:** 2026-06-27
- **Deciders:** @stephane-segning

## Context and Problem Statement

The `poller` role was named for its first job — [ADR-0035](0035-review-feedback-signal.md): a single-replica
loop that *reads* 👍/👎 reactions on the comments we posted and reconciles them into `review_feedback`.
Since then it has grown a second, opposite job: [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md)
made it **post** the failure notice for an uncatchable kill (the keyless dispatcher can't, so the
key-holding singleton does). And [ADR-0059](0059-reconciler-owns-all-github-egress.md) goes further —
the role becomes the **sole writer to GitHub** for everything outbound.

At that point "poller" is not just incomplete, it's actively misleading: a role called *poller* that
posts reviews, replies, reactions, and notices is exactly the kind of name that makes the next
maintainer mistrust the code. The name describes the inbound half of a now-bidirectional role.

## Decision

Rename the role **`poller` → `reconciler`** everywhere it appears:

- the role string in the `match role` dispatch (`services/control-plane/src/main.rs`) and the
  `run_poller` → `run_reconciler` entry point;
- the Deployment name + `args` in **ai-helm**, and the value in **ai-helm-values**;
- metrics labels and log targets;
- ADR / doc references.

"Reconciler" names what the role actually does: **bidirectional GitHub reconciliation** — it pulls
GitHub state in (reactions → `review_feedback`) and pushes our intended state out (the
[ADR-0059](0059-reconciler-owns-all-github-egress.md) egress outbox → GitHub). "Reconcile" is already
literal in the code (`reconcile_comment_feedback`); the name now matches the seam.

**Safe rollout ordering** (the role string is a deploy contract). Ship a binary that accepts **both**
`poller` and `reconciler` first; flip the Deployment `args` to `reconciler`; then drop the `poller`
alias in a later release. A binary that only knows the new string would `bail!("unknown role")` against
a Deployment still passing the old one (or vice-versa) during the rollover window.

## Consequences

- **Good:** the name matches behavior, and ADR-0059 reads naturally ("the reconciler reconciles"
  beats "the poller posts"). Removes a standing source of confusion before the role's responsibility
  grows again.
- **Cost:** a cross-repo but mechanical rename (control-plane code + ai-helm + ai-helm-values), plus the
  two-step alias rollout above so there's no window where the Deployment's role string and the binary
  disagree.
- **Scope:** identity/naming only — no behavioral change ships in this ADR. The behavior change is
  [ADR-0059](0059-reconciler-owns-all-github-egress.md); this rename is its stage 1 so the rest of that
  work lands against the right name.

## References

- [ADR-0035](0035-review-feedback-signal.md) — the role's original (inbound) job: read reactions.
- [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md) — gave it its first outbound job.
- [ADR-0059](0059-reconciler-owns-all-github-egress.md) — makes it the sole GitHub egress (the reason
  the rename is worth doing now).
- [ADR-0002](0002-rust-control-plane-trust-boundary.md) — the role split and where the App key lives.
