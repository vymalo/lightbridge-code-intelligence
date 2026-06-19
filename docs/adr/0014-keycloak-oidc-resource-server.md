# ADR-0014: Keycloak OIDC — web as OIDC client, control plane as resource server

- **Status:** Accepted
- **Date:** 2026-06-19
- **Supersedes:** [ADR-0007](0007-better-auth-rust-backend-plugin.md)

## Context and Problem Statement

[ADR-0007](0007-better-auth-rust-backend-plugin.md) had the web app use **better-auth** with a
custom `rust-backend` plugin that POSTed credentials to the control plane, which verified them
against its own user store. That makes us responsible for credential storage, password hashing,
sessions, and a bespoke auth surface — and it does not give us SSO. We would rather **manage no
authentication ourselves** and reuse a standards-based identity provider.

## Decision Drivers

- Manage no credentials, sessions, or password storage in our own code.
- Standards-based **OIDC / OAuth2** so external SSO (enterprise IdPs) is a configuration change.
- One token standard that the gateway authorization layer (Envoy + **Authorino**) can validate too.
- Keep the web app framework-light (no bespoke auth framework).

## Considered Options

- **Keycloak OIDC; web = client, control plane = resource server** — Keycloak owns users, login,
  and token issuance. The Next.js app runs the Authorization-Code + PKCE flow as an OIDC client; the
  Rust control plane validates the resulting RS256 JWTs against Keycloak's JWKS.
- **better-auth + first-party credentials (ADR-0007)** — we own the identity store. Rejected: more
  code to secure, no SSO.
- **A hosted IdP (Auth0/Entra/Okta) directly** — viable, but Keycloak is self-hostable, fits the
  banking-grade/adorsys context, and (being OIDC) can federate to those IdPs later anyway.

## Decision Outcome

Chosen option: **Keycloak as the OIDC provider; web app as an OIDC client; control plane as a pure
OAuth2 resource server.** The web client uses PKCE; it is a **public** client in local dev (no
committed secret) and a **confidential** client in production (`OIDC_CLIENT_SECRET` from a secret
store).

- **Tokens:** RS256, validated via the provider's **JWKS** (`iss` / `aud` / `exp`). No shared
  secrets, so external IdPs work unchanged.
- **Web app (`apps/web`):** Authorization-Code + PKCE in Node route handlers (`/api/auth/login`,
  `/api/auth/callback`, `/api/auth/logout`) using `openid-client`; the access token rides in an
  httpOnly cookie; `middleware.ts` validates it with `jose` (Edge runtime) on protected routes.
- **Control plane (`services/control-plane`):** validates bearer JWTs (`src/jwt.rs`) and reads
  identity from the claims (`sub`, `email`, …). **No user store, no token issuance** — the argon2
  `/auth/verify` path and the `AuthUser`/`AuthSession` schema from ADR-0005/0007 are removed.
- **Issuer-agnostic by config:** `OIDC_ISSUER` points at the Keycloak dev container locally and at
  any OIDC IdP in production — **SSO is added by configuration, not code.**
- **Refresh:** deferred. Short-lived access tokens; re-login on expiry. A follow-up ticket covers
  refresh-token rotation.

### Production / SSO

The same `OIDC_ISSUER` + JWKS validation runs in production against the real IdP (Keycloak, or any
OIDC provider it federates to). Because the gateway authZ layer (Envoy + Authorino) validates the
*same* JWTs via JWKS, **authentication and authorization converge on one token standard**. Key
rotation is handled by the IdP's JWKS (`kid`-based); verifiers refresh automatically. In Kubernetes,
the token `iss` must match the configured issuer across browser and services — mind the in-cluster
vs public hostname split.

### Consequences

- Good, because we store no credentials and manage no sessions; identity is the IdP's job.
- Good, because SSO/enterprise federation is a config change, and Authorino reuses the same tokens.
- Good, because the web app is framework-light (`openid-client` + `jose`, no auth framework).
- Bad, because local dev now depends on a Keycloak container (added to `compose.yaml`).
- Neutral, because revocation is bounded by token lifetime until refresh/rotation lands (follow-up).
