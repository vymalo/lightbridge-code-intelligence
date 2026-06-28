# ADR-0063: CLI-only repository approval (retire the web approval gate)

- **Status:** Proposed
- **Date:** 2026-06-28
- **Deciders:** @stephane-segning

## Context and Problem Statement

The web console's **last unique function** is the repository **approval gate** ([ADR-0023](0023-db-backed-rbac.md),
admin governance epic #75/#89): before a repository is indexed or reviewed, an admin must approve it
(`repo:approve` / `repo:deny`). Everything else the console did — runs, insights, transcripts, feedback —
is moving to Grafana ([ADR-0064](0064-observability-via-grafana-behind-caddy-oauth2.md)). Maintaining a
full Next.js + OIDC single-page app ([ADR-0006](0006-nextjs-app-router-web-ui.md),
[ADR-0027](0027-daisyui-design-system.md)) just to flip an approve/deny toggle is disproportionate.

**Can approvals move to a CLI, so `apps/web` can be retired entirely?**

## Decision Drivers

- **Shrink the maintained surface** — retire a Next.js app, its OIDC SPA client, and the daisyUI design system.
- **Reuse the existing trust boundary + authz** ([ADR-0002](0002-rust-control-plane-trust-boundary.md),
  [ADR-0014](0014-keycloak-oidc-resource-server.md), [ADR-0023](0023-db-backed-rbac.md)) — do **not** invent a new auth path.
- **Auditability** — who approved/denied what, and when.
- **Single-operator ergonomics** — the operator runs their own infra and lives in a terminal; a GitOps-leaning shop.

## Considered Options

- **Option A — CLI over the existing OIDC-gated control-plane endpoints (device-code flow).** A small binary
  authenticates via Keycloak's OAuth **device authorization grant**, then calls the same per-capability
  endpoints the web used (`repo:read` / `repo:approve` / `repo:deny`). No new server surface; the authz model
  is unchanged.
- **Option B — GitOps-declared approvals.** The approved-repo set lives declaratively in `ai-helm-values`
  (or a small CRD) and the control plane reconciles it. No interactive auth, fully audited via git history,
  matches the GitOps ethos — but approval becomes a PR/merge, not a one-liner.
- **Option C — Status quo.** Keep the web console solely for approvals.

## Decision Outcome

Chosen option: **A — a CLI using the OAuth device-code flow against the existing permission-gated
endpoints**, with **B noted as a strong complement** (and the likely long-term home if approvals should be
declarative). A best satisfies the drivers: it reuses [ADR-0023](0023-db-backed-rbac.md) authz verbatim
(no new trust path), keeps approval an explicit, audited human action, and — together with
[ADR-0064](0064-observability-via-grafana-behind-caddy-oauth2.md) — lets `apps/web` be **deleted**.

This ADR is **Proposed** — open for discussion, especially A-vs-B (imperative CLI vs declarative GitOps).

### Consequences

- **Good** — `apps/web` (Next.js + OIDC SPA + daisyUI) can be retired once observability is on Grafana
  (ADR-0064); one fewer language/stack to maintain.
- **Good** — approval stays a `repo:approve`-gated, attributable action; the control plane records the
  approving identity (from the token's permission claim) + timestamp.
- **Bad / tension** — a CLI is an **imperative** state change, which sits awkwardly next to this project's
  **GitOps-declarative** norm (prod mutation via merge, not exec). Mitigated by Option B, or by having the
  CLI write an auditable record; worth resolving in discussion.
- **Bad** — must build + distribute the binary and handle device-code token caching/refresh.
- **Neutral** — needs a stable admin API surface (`list` / `approve` / `deny`); the web already consumes
  one, so the CLI mostly reuses it.

## Pros and Cons of the Options

### Option A — device-code CLI over existing endpoints
- Good — reuses ADR-0014/0023 authz; zero new trust path; approval stays explicit + audited.
- Good — tiny surface; a single binary the operator already wants.
- Bad — imperative; another artifact to build/ship; device-code UX + token storage.

### Option B — GitOps-declared approvals
- Good — declarative, fully git-audited, no interactive auth, matches the deploy model.
- Bad — approval is now a PR/merge round-trip; needs a reconcile loop + drift handling.

### Option C — keep the web for approvals
- Good — nothing to build.
- Bad — keeps an entire Next.js + OIDC app alive for one toggle; the thing we're trying to retire.

## More Information

- Retires the web: [ADR-0006](0006-nextjs-app-router-web-ui.md), [ADR-0027](0027-daisyui-design-system.md).
- Authz reused: [ADR-0014](0014-keycloak-oidc-resource-server.md), [ADR-0023](0023-db-backed-rbac.md).
- Companion: [ADR-0064](0064-observability-via-grafana-behind-caddy-oauth2.md) (observability → Grafana).
- Admin governance origin: epic #75 / permission authz #89.
