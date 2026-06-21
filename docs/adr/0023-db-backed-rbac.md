# ADR-0023: Permission-based authorization from a custom token claim

- **Status:** Accepted
- **Date:** 2026-06-21
- **Deciders:** Stephane Segning Lambou

## Context and Problem Statement

Authorization today is one hard-coded check: the `Admin` extractor requires the Keycloak **realm
role** `lci-admin` (`realm_access.roles`) to reach `/admin/*` (ADR-0014, Epic #75). We don't want to
depend on realm roles, and a single "is-admin" boolean doesn't scale.

How should the control plane authorize actions? An earlier draft of this ADR proposed managing
roles → permissions in our own database. **Refined per the owner: there are no role names on our
side — the token carries a flat list of *permissions* directly**, and we simply enforce them. The
IdP / Envoy AI Gateway owns whatever role→permission policy it likes; we consume the result.

## Decision Drivers

- **Permissions in the token, not roles** — the IdP asserts the caller's *permissions*; we don't
  model or store roles. Keep **audience (`aud`) verification**.
- **Configurable claim** — the permissions claim path is config, so the IdP/token shape is swappable.
- **Fine-grained + least privilege** — gate each action on a specific permission.
- **No new datastore** — nothing to persist; authz is a pure function of the verified token.

## Decision Outcome

**The token carries a list of permissions under a configurable claim; the control plane verifies the
JWT (signature / issuer / `aud` / expiry) and authorizes each request on the permission it requires.
No roles, no RBAC tables, no admin policy UI** — the IdP/gateway maps roles→permissions upstream.

This supersedes the earlier "DB-backed RBAC" framing (simpler; the owner confirmed permissions come
straight from the token).

### Token → permissions

1. Verify the JWT — unchanged (incl. `aud`).
2. Read the caller's **permissions** from a configurable claim path **`PERMISSIONS_CLAIM`** (default
   `permissions`; dotted paths supported for nested claims, e.g. `code_intelligence.permissions`).
   The claim value is a JSON array of permission strings.
3. Each handler requires a **permission**; 403 if it isn't in the caller's set.

### Permission catalogue (confirmed)

| Permission | Gates |
|---|---|
| `repo:read` | `GET /repositories`, `GET /admin/repositories` |
| `repo:approve` | `POST /admin/repositories/{id}/approve` |
| `repo:deny` | `POST /admin/repositories/{id}/deny` |
| `task:read` | `GET /tasks`, `GET /tasks/{id}` |
| `task:logs` | `GET /tasks/{id}/logs` (web SA route) |
| `review:read` | `GET /tasks/{id}/review` |
| `rbac:manage` | reserved (future policy/admin surface) |

These are documented as code constants (a catalogue), not a table. `GET /me` returns the caller's
effective permissions so the web can show/hide affordances; the control plane stays the enforcement
point.

### Authorization seam (Rust)

Replace the `Admin` extractor with a **`Caller`** extractor that carries the verified `Claims` + the
permission set parsed from the configured claim, plus `require(perm) -> Result<(), AuthError>`
(403 `Forbidden` on miss). Read-only `/tasks*` and `/repositories` move from "any valid token" to
their `*:read` permissions; `/admin/*` move from the realm-role check to `repo:approve` / `repo:deny`.

### Web

`SessionClaims` reads the same permissions claim; `isAdmin` becomes `hasPermission(claims, perm)`
(claim path from `PERMISSIONS_CLAIM`, default `permissions`). The Approvals nav/screen shows when the
caller has `repo:approve`; server actions authorize on the permission. The control plane remains the
real gate.

### Migration / cutover

The IdP must now emit the permissions claim. Until it does, a token without the claim has **no
permissions** → `/admin/*` 403s and the read endpoints 403 (fail-closed). **Operator step:** configure
Keycloak/the gateway to emit `permissions` (e.g. mapping the existing `lci-admin` users to the full
catalogue). Optionally, a transitional `PERMISSIONS_FALLBACK_ALL_FOR_CLAIM` could grant all perms to
holders of a named legacy claim — deferred unless needed.

### Consequences

- Good — dead simple: authz is a pure function of the verified token; no DB, no policy UI to build.
- Good — IdP-agnostic (configurable claim) + `aud` still enforced; per-capability least privilege.
- Good — the web renders capability-aware UI from `/me`.
- Bad — role→permission policy lives in the IdP, not here (by design — the owner wants that); we can't
  show/manage it in-app.
- Neutral — **cutover requires the IdP to emit the claim**; fail-closed until then.
