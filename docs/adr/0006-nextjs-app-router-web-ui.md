# ADR-0006: Next.js (App Router) for the web UI

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Lightbridge needs a web console (`apps/web`) for operators: repository onboarding, task history,
index status, and audit trails. We need a frontend framework that fits the pnpm + Turborepo
monorepo, supports server-side rendering and server components, and integrates cleanly with our
chosen auth library.

## Decision Drivers

- First-class server components and server-side data fetching
- Mature ecosystem and routing for an operator console
- Clean fit with the monorepo and with better-auth
- Familiarity and hiring pool

## Considered Options

- **Next.js (App Router)** — React server components, file-based routing, strong SSR, broad
  ecosystem.
- **SPA (Vite + React Router)** — simpler build, but no SSR/server components out of the box.
- **Remix / other meta-frameworks** — viable, but less alignment with our existing tooling.

## Decision Outcome

Chosen option: **Next.js with the App Router.** It gives server components and SSR, fits the
Turborepo workspace as `apps/web`, and runs the [Keycloak OIDC client flow](0014-keycloak-oidc-resource-server.md).

### Consequences

- Good, because server components keep sensitive fetching on the server and simplify auth flows.
- Good, because it slots into the monorepo and shared `packages/tsconfig` cleanly.
- Bad, because the App Router has a steeper learning curve than a plain SPA.
- Neutral, because the web UI is an operator surface, not the system of record (the control plane
  is).
