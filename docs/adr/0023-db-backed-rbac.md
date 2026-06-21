# ADR-0023: DB-backed RBAC — roles from a custom token claim, permissions in Postgres

- **Status:** Proposed
- **Date:** 2026-06-21
- **Deciders:** Stephane Segning Lambou

## Context and Problem Statement

Authorization today is a single hard-coded check: the `Admin` extractor requires the Keycloak
**realm role** `lci-admin` (`realm_access.roles`) to reach `/admin/*` (ADR-0014, Epic #75). Two
problems: (1) we don't want to depend on Keycloak **realm roles** — roles should come from a
**custom claim** so the IdP is swappable and the claim is ours to shape; (2) a single boolean
"is-admin" doesn't scale — we want **fine-grained permissions** and the ability to manage which
role grants what **without a redeploy**.

How should the control plane authorize actions, given roles arrive in the token but the
role→capability policy should be ours to manage at runtime?

## Decision Drivers

- **Custom claim, not realm roles** — the IdP only asserts *who has which roles*; the claim path is
  configurable. Keep **audience (`aud`) verification** (a token for another service must not work here).
- **Fine-grained + runtime-manageable** — gate each action on a **permission**; map roles→permissions
  in **Postgres**, editable via an admin UI (no redeploy to change policy).
- **Least privilege + auditability** — distinct permissions per capability; changes are visible.
- **Smooth migration** — existing `lci-admin` admins keep working.

## Considered Options

- **A. DB-backed RBAC**: token carries *roles* (custom claim); Postgres holds *permissions*, *roles*,
  and the *role→permission* mapping; actions gated on permissions; admin UI manages the mapping.
- **B. Custom claim, simple role check**: swap `realm_access` for a custom claim, keep "has role X".
- **C. Permissions directly in the token**: the IdP asserts permissions; no DB.

## Decision Outcome

Chosen: **A — DB-backed RBAC.** The token asserts identity + roles (via a configurable custom claim,
with `aud` still verified); the control plane resolves those roles to a **permission set** from
Postgres and authorizes each request on the **permission** it needs. The role→permission policy is
data, managed by admins at runtime.

### Token → roles → permissions

1. Verify the JWT (signature/issuer/`aud`/expiry) — unchanged.
2. Read the caller's **roles** from a configurable claim path **`ROLES_CLAIM`** (default `roles`;
   dotted paths supported for nested claims, e.g. `code_intelligence.roles`). ⚠️ **Open question —
   confirm the exact claim your tokens emit.**
3. Resolve `roles → permissions` via `role_permissions` (one query; cached briefly).
4. Each handler requires a **permission** (e.g. `repo:approve`); 403 if the caller lacks it.

### Schema (proposed migration `0010_rbac.sql`)

```sql
CREATE TABLE permissions (
    key         text PRIMARY KEY,          -- e.g. 'repo:approve'
    description text NOT NULL DEFAULT ''
);
CREATE TABLE roles (
    name        text PRIMARY KEY,          -- matches a role string in the token claim
    description text NOT NULL DEFAULT '',
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE role_permissions (
    role       text NOT NULL REFERENCES roles (name)        ON DELETE CASCADE,
    permission text NOT NULL REFERENCES permissions (key)   ON DELETE CASCADE,
    PRIMARY KEY (role, permission)
);

-- Seed the permission catalogue.
INSERT INTO permissions (key, description) VALUES
  ('repo:read',    'View repositories + the approval queue'),
  ('repo:approve', 'Approve a pending repository'),
  ('repo:deny',    'Deny / disable a repository'),
  ('task:read',    'View runs/tasks'),
  ('task:logs',    'Stream a run''s logs'),
  ('review:read',  'View persisted review output'),
  ('rbac:manage',  'Manage roles, permissions, and their mapping')
  ON CONFLICT DO NOTHING;

-- Default roles.
INSERT INTO roles (name, description) VALUES
  ('admin',      'Full access incl. RBAC management'),
  ('maintainer', 'Manage repositories + read everything'),
  ('viewer',     'Read-only')
  ON CONFLICT DO NOTHING;

-- Default mapping (admin = all; maintainer = repo mgmt + reads; viewer = reads).
INSERT INTO role_permissions (role, permission)
  SELECT 'admin', key FROM permissions
UNION ALL SELECT 'maintainer', key FROM permissions
            WHERE key IN ('repo:read','repo:approve','repo:deny','task:read','task:logs','review:read')
UNION ALL SELECT 'viewer', key FROM permissions
            WHERE key IN ('repo:read','task:read','review:read')
  ON CONFLICT DO NOTHING;
```

**Migration bridge:** also seed a role named after the current `ADMIN_ROLE` (default `lci-admin`)
granted all permissions, so admins whose token already carries `lci-admin` keep working the moment
this ships — no IdP change required first.

### Permission ↔ endpoint map

| Endpoint | Permission |
|---|---|
| `GET /tasks`, `/tasks/{id}` | `task:read` |
| `GET /tasks/{id}/logs` (web SA route) | `task:logs` |
| `GET /tasks/{id}/review`, `/repositories` | `review:read` / `repo:read` |
| `GET /admin/repositories` | `repo:read` |
| `POST /admin/repositories/{id}/approve` | `repo:approve` |
| `POST /admin/repositories/{id}/deny` | `repo:deny` |
| `GET/POST /admin/rbac/*` (manage policy) | `rbac:manage` |

### Authorization seam (Rust)

Replace the `Admin` extractor with a permission-checked one — e.g. `Caller` carrying the resolved
permission set + a `require(perm)` helper, or a typed `RequirePermission<P>` extractor. `me` returns
the caller's effective permissions so the web can show/hide affordances; the control plane stays the
enforcement point.

### Admin UI

A `/dashboard/admin/roles` screen (gated by `rbac:manage`): the **role × permission matrix** —
view roles, toggle permissions per role, add/remove roles. Backed by `/admin/rbac/*` endpoints.

### Consequences

- Good — policy is data: change who-can-do-what without a redeploy; least-privilege by capability.
- Good — IdP-agnostic (configurable claim) + `aud` still enforced; clean migration via the seeded
  `lci-admin` role.
- Good — the web can render capability-aware UI from the caller's effective permissions.
- Bad — more moving parts (3 tables, a policy cache, an admin screen) than a boolean check.
- Neutral — **user→role** assignment stays in the IdP (the token claim); only **role→permission** is
  in our DB. (A future ADR could add DB-side user→role grants if we ever need local assignment.)

## Open questions (please confirm before I build)

1. **Claim path** — what does your token actually carry? (`roles`, `permissions`, a nested
   `code_intelligence.roles`, …) → sets the `ROLES_CLAIM` default.
2. **Permission set** — is the catalogue above right, or split finer (e.g. `task:cancel`)?
3. **Role names** — keep `admin`/`maintainer`/`viewer` as defaults, or use your own (e.g. `lci-admin`
   only)?
