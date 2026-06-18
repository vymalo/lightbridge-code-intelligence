# ADR-0007: better-auth for web auth via a rust-backend delegation plugin

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

The Next.js web app ([ADR-0006](0006-nextjs-app-router-web-ui.md)) needs authentication. We already
have a standalone, portable Rust backend (the control plane) that can own the user store and
credential verification. We want web authentication that delegates the actual credential check to
that backend rather than duplicating an identity store in the frontend — and we must not conflate
this with gateway authorization.

## Decision Drivers

- Reuse our own portable Rust authentication surface
- Keep the credential store in one place (the control plane)
- A clean, typed integration point in the Next.js app
- A clear separation between authentication (authN) and authorization (authZ)

## Considered Options

- **better-auth with a custom `rust-backend` plugin** — better-auth manages sessions in Next.js; a
  custom plugin POSTs credentials to `${AUTH_BACKEND_URL}/auth/verify` on our Rust backend.
- **A hosted IdP (e.g. Auth0/Clerk)** — less code, but introduces an external dependency and does
  not reuse our portable backend.
- **Hand-rolled auth in Next.js** — maximal control, minimal leverage, easy to get wrong.

## Decision Outcome

Chosen option: **better-auth with a custom `rust-backend` plugin** that delegates credential
verification to our own standalone Rust backend at `POST ${AUTH_BACKEND_URL}/auth/verify`.

**This decision is about authentication (authN) only.** It is explicitly **distinct** from gateway
**authorization (authZ)**, which is handled by **Envoy + Authorino** together with the separate
[`ADORSYS-GIS/lightbridge-authz`](https://github.com/ADORSYS-GIS/lightbridge-authz) component.
`lightbridge-authz` is **not** this project's auth backend, and our better-auth Rust backend is
**not** the gateway authorizer.

### Consequences

- Good, because credential verification stays in our portable Rust backend, not the frontend.
- Good, because authN and authZ remain cleanly separated and independently evolvable.
- Bad, because the `rust-backend` plugin and the `/auth/verify` contract must be maintained and
  tested (see `services/control-plane/tests/auth_contract.rs`).
- Neutral, because the AuthUser store lives in the control-plane schema
  ([ADR-0005](0005-cratestack-schema-first-control-plane.md)).
