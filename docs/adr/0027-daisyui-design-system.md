# ADR-0027: Adopt daisyUI (dracula theme) as the component layer

- **Status:** Accepted
- **Date:** 2026-06-22

## Context and Problem Statement

[ADR-0015](0015-web-console-design-language.md) gave the console a **hand-rolled** design system:
Tailwind v4 with a bespoke token set (`--color-surface`, `--status-*`, …) and in-repo, shadcn-style
components (`status-pill`, `Card`, the `SettingsSection`/disclosure/table primitives from
[ADR-0024](0024-web-console-redesign-v2.md)). It's cohesive, but every surface carries a lot of
bespoke utility-class strings, and every new component is hand-built — a maintenance cost that grows.

We want the design system to be **mostly off-the-shelf** so future work is "compose a `btn`/`card`/
`badge`", not "hand-assemble ten utilities" — minimizing bespoke CSS for long-term maintainability,
on **Tailwind + daisyUI** (the project already runs Tailwind v4; daisyUI v5 is CSS-first on v4).

## Decision Drivers

- **Maintainability first:** fewer bespoke classes; reach for daisyUI component classes (`btn`,
  `card`, `badge`, `menu`, `table`, `tabs`, `input`, `select`, `dropdown`) before hand-rolling.
- **Keep the *philosophy* of ADR-0015** (restraint, semantic-status color, calm density) while
  swapping the *mechanism* (bespoke tokens/components → daisyUI tokens/components).
- A single, named theme we can tweak centrally rather than scattered CSS vars.
- Don't regress the work already shipped (ADR-0024 surfaces) — migrate, don't rewrite blindly.

## Decision Outcome

Adopt **daisyUI v5** as the component layer on Tailwind v4, themed with the provided **`dracula`**
theme. This **supersedes the component/token *mechanism* of [ADR-0015](0015-web-console-design-language.md)**
(not its principles).

- **Theme:** register `dracula` (the operator-supplied block) via `@plugin "daisyui/theme" { … }` in
  `globals.css`; enable daisyUI with `@plugin "daisyui"`. daisyUI semantic colors become the palette:
  `base-100/200/300` (surfaces), `base-content` (text), `primary`/`secondary`/`accent`, `neutral`, and
  `info`/`success`/`warning`/`error` (+ `*-content`). Radii/sizes come from the theme
  (`--radius-box/field/selector`).
- **Status model → daisyUI semantics:** map the ADR-0016 status variants to daisyUI colors —
  `success`→success, `error`→error, `warn`→warning, `active`→info (or primary), `pending`/`muted`→
  neutral. The `status-pill` becomes a daisyUI `badge` with the semantic color; the hand-rolled
  `.status-*` CSS is removed.
- **Components:** migrate surface-by-surface (like ADR-0024): `Card`→`card`, buttons→`btn`,
  nav→`menu`, the runs table→daisyUI `table`, settings rows→`fieldset`/`label`, the ⌘K palette keeps
  `cmdk` but restyled with daisyUI tokens, etc. Delete bespoke CSS as each surface moves.
- **Theme scope:** `dracula` is **dark** (`color-scheme: dark`). This shifts the console from
  ADR-0015's light-default + dark to **dark-first** (consistent with "our users live in dark IDEs").
  A light daisyUI theme can be added later via `data-theme` if we want the toggle back — flagged, not
  in the first cut.

### Migration plan (incremental, each its own PR)

1. **Setup + tokens:** add daisyUI, register the `dracula` theme, map status→semantic colors; keep the
   old utilities working so nothing breaks mid-migration.
2. **Primitives:** `Card`, `StatusPill`/badge, buttons, inputs/selects → daisyUI.
3. **Surfaces:** runs list/table, run detail, settings, repositories, overview/insights, shell/nav.
4. **Cleanup:** remove the bespoke `@theme`/`@layer` tokens + `.status-*` CSS once nothing references
   them. ADR-0024's *patterns* (timeline, disclosure rows, insights) stay; only their *styling* moves.

### Consequences

- Good: far less bespoke CSS; new UI is composition of documented components; one theme block to tune;
  faster, more maintainable UI work — the stated goal.
- Bad: a dependency (daisyUI) + a re-skin of surfaces just built in ADR-0024 (real rework), and a
  shift to **dark-first** (the light theme goes unless we add a light daisyUI theme).
- Neutral: still Tailwind underneath; daisyUI is class-only (no runtime JS), so it composes with our
  existing Tailwind utilities where a component doesn't fit.

### References

- Supersedes the component/token mechanism of [ADR-0015](0015-web-console-design-language.md); keeps
  its philosophy + [ADR-0016](0016-dashboard-information-architecture.md) IA and
  [ADR-0024](0024-web-console-redesign-v2.md) patterns. daisyUI v5 (CSS-first, Tailwind v4).
- Theme: operator-supplied `dracula` block (`@plugin "daisyui/theme"`).
