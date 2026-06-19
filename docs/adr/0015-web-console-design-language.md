# ADR-0015: Web console design language & component system

- **Status:** Accepted
- **Date:** 2026-06-19

## Context and Problem Statement

The web app ([ADR-0006](0006-nextjs-app-router-web-ui.md)) currently has only hand-rolled minimal
CSS — enough for the auth skeleton, but not for the real console we're about to build (a task-run
dashboard and more, see [ADR-0016](0016-dashboard-information-architecture.md)). Before writing UI
we want a deliberate design language and a component foundation, grounded in real references rather
than invented from scratch.

Design references pulled via Refero (developer-tool / observability consoles):
shadcn/ui (`https://ui.shadcn.com`), Linear (`linear.app`), Supabase, Checkly, Depot, Cursor,
Vercel. Common thread: **restrained, monochrome, information-dense, borders over shadows, semantic
status accents** — a calm "command-center" feel, not a decorated marketing site.

## Decision Drivers

- Calm, considered, low-decoration UI (house design philosophy) — restraint reads as quality.
- Developer-tool density: many rows/states scannable at a glance.
- Build speed + accessibility: don't hand-roll primitives (menus, dialogs, tabs).
- Composition over re-implementation; a token system we can theme (light + dark).

## Considered Options

- **shadcn/ui (Tailwind + Radix primitives), copy-in components** — own the code, accessible
  primitives, matches the monochrome/technical references, no heavy runtime.
- **A batteries-included component library (MUI / Mantine / Chakra)** — faster start but opinionated
  look, heavier, harder to make feel restrained/bespoke.
- **Keep hand-rolled CSS** — full control, but we'd reinvent accessible primitives and drift.

## Decision Outcome

Chosen: **shadcn/ui on Tailwind CSS + Radix primitives.** It fits the reference aesthetic, keeps the
component code in-repo (composition-friendly), and gives accessible primitives for free.

**Design tokens / rules:**
- **Palette:** monochrome neutral scale as the base; color reserved for **semantic status** only.
  Status tokens: `pending` (neutral/gray), `active` (blue), `success` (green), `error` (red),
  `warn` (amber), `muted` (cancelled/disabled). One restrained brand accent for primary actions.
- **Type:** Inter (UI) + a monospace (code, SHAs, logs). Compact, strong hierarchy; medium-weight
  headings rather than heavy bold (Linear-like).
- **Surfaces:** flat; depth via **thin borders and tonal shifts, not drop shadows**. Radius `8px`
  cards, capsule (`9999px`) for pills/filters/badges.
- **Density:** compact, table/row-oriented; generous vertical rhythm within a contained max-width.
- **Theme:** light default with full **dark mode** (the references skew dark; our users live in
  dark IDEs). Tokenize so both ship from one set.
- **Decoration budget:** none gratuitous — no gradients/glows that don't earn their keep.

### Consequences

- Good: cohesive, accessible, fast to build; matches grounded references; themable light/dark.
- Bad: introduces Tailwind + shadcn tooling to `apps/web` (currently plain CSS) — a one-time setup.
- Neutral: component code lives in-repo (we maintain it), which is the point of shadcn.

### References

- Styles: shadcn/ui `c14c0a94…`, Linear Changelog `11d3e58a…`, Supabase `28aeb534…`,
  Checkly `0a2ad49e…`, Depot `707c2922…` (Refero).
- Consumed by [ADR-0016](0016-dashboard-information-architecture.md) (dashboard IA) and the later
  web-console implementation.
