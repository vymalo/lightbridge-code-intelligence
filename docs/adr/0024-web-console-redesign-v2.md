# ADR-0024: Web console redesign v2 — surfaces, navigation, table & insights

- **Status:** Proposed
- **Date:** 2026-06-21

## Context and Problem Statement

The console was built to the design language ([ADR-0015](0015-web-console-design-language.md)) and
information architecture ([ADR-0016](0016-dashboard-information-architecture.md)) we set before
writing any UI. We've now lived with the result through epic #75 (admin governance) and epic #89
(quick wins). It is clean and honest, but several surfaces are thinner than the product now deserves,
and we've decided to go beyond a polish pass into a **fuller v2**:

- **Settings** is a single card with one GitHub-App link — it reads like a stub, and more settings
  are coming (permissions/RBAC, attribution, namespaces).
- **Run detail logs** render as an undifferentiated stream; no level filter, no copy/download, and the
  #103 finding format (`<LEVEL>: <title>` → explanation → suggestion → resources) is shown flat
  rather than as something you scan and expand.
- **The runs list** is a flat stack of rows with no time grouping, no table/scan view, and only a
  partial filter bar — hard to use once a repo has history.
- **Overview** stat cards are flat counts with no hierarchy, drill-through, or trend.
- **The shell** has a flat nav and a top bar that only holds user+sign-out — no global search or
  keyboard navigation, which power users of a dev console expect.
- There is **no insights surface** — operators can't see review volume, pass/fail rate, or latency
  trends without leaving for Grafana.

This v2 keeps the [ADR-0015](0015-web-console-design-language.md) **contract intact**: monochrome
neutral base, **color reserved for semantic status only**, depth via **thin borders and tonal shifts,
not shadows**, Inter + mono, `8px` cards / capsule pills, one token set for light+dark. "Fuller" means
**richer patterns, navigation, and a real data/insights layer within that contract** — it raises the
ceiling, it does not repaint the walls.

### References (Refero, current dev-tool / agent consoles; platform web)

Carried from v1 (still the base): **Cursor — Cloud Agents** `04df139c-3480-4da4-8f00-fcfaf4509bbd`
(sectioned settings: uppercase label, `label + description` left / control right, hairline dividers);
**Lovable — Cloud Logs** `f9ee3054-455d-485f-bbc1-d1e3b7b7ea31` (filter bar + expandable rows +
copy/download/raw-inspect); **Doppler — Activity Logs** `1d2903be-80cf-4804-af0b-355115053e85`
(day-grouped timeline rail, entity links, quiet meta line).

New for the bold scope:
- **Appwrite — Auth users table** `b966c079-334e-47b6-82da-d65f6b46cbd0`: a clean admin **data table**
  — search, column-config, mono copyable IDs, status badges, `N per page · Prev/Next`. The model for
  the runs table view.
- **Dub — Analytics** `ca73bf7d-b644-4b08-9961-643b70fed7e0`: a restrained **insights** layout — a
  date-range popover + one hero time-series chart with a single accent, then breakdown cards with
  inline bars. The model for the Insights surface (and proof that "charts" can stay minimal).
- **Raycast — command palette** `370ddbce-beea-405c-86d5-f43e61f6a453`: the canonical **⌘K** — dark
  modal, search, muted group labels, rows with icon + label + ⏎ hint, keyboard-first.

Visual north star: **Linear Changelog** `11d3e58a-…` (cited by ADR-0015) — its description is
effectively our token set, confirming v2 stays in lineage.

## Decision Drivers

- Restraint is the house rule — "fuller" must mean *clearer hierarchy, richer patterns, and genuine
  capability*, never decoration. The ADR-0015 contract is non-negotiable.
- Each surface earns a concrete, grounded pattern from a real console, not an invention.
- Reuse what we have (the #103 finding format, the run logs stream, the `TaskStatus` map, the existing
  task-list API) — render it better and aggregate it, don't re-model it.
- New capability must not smuggle in heavy dependencies or break the monochrome/borders aesthetic.
- Ship incrementally: several small, independently reviewable and revertible PRs, not a big-bang rewrite.

## Decision Outcome

Adopt a **v2 across the whole console**, grounded surface-by-surface. Shared primitives are extended,
not forked. Three sub-decisions keep the new capability on-contract:

- **Charts = hand-rolled SVG + CSS bars, not a chart library.** The only true chart is one runs-over-time
  series; render it as a small inline SVG line/area in a single accent. Breakdown cards (by repo, by
  outcome) use CSS bar rows (Dub pattern). This avoids adding Recharts/visx, keeps full token control,
  and stays monochrome. *Revisit only if the insights surface grows past a couple of chart types.*
- **Command palette = `cmdk`** (the shadcn-native, dependency-light primitive endorsed by
  [ADR-0015](0015-web-console-design-language.md)'s shadcn choice). No bespoke palette.
- **Insights data = client-side aggregation MVP first, a control-plane endpoint later.** The current API
  returns the task list; v1 of Insights aggregates that in the web tier (counts, rates, a daily bucket).
  A dedicated control-plane aggregation endpoint is a flagged follow-up for when volume outgrows
  fetch-all. The runs **table** likewise sorts/filters/paginates client-side first, server-side later.

### Shared additions (within the contract)

`SettingsSection` (uppercase label + label/description/control rows + hairline dividers, *Cursor*);
**meta line** (`·`-separated quiet facts, *Doppler*); **filter bar** (search + selects + actions,
*Lovable/Appwrite*); **disclosure row** (chevron + summary/detail, *Lovable*); **data table**
(sortable headers, mono IDs, per-page footer, *Appwrite*); **inline-bar breakdown card** + **SVG
sparkline/series** (*Dub*); **command palette** (`cmdk`, *Raycast*). No new colors, shadows, or radii;
motion stays at the current budget plus reduced-motion-safe disclosures.

### Per surface

1. **Runs list (Doppler timeline + Appwrite table) — first PR.** Default **timeline** view: rows grouped
   by day on a thin rail; each run = status pill + trigger title + repo/branch entity links + a meta
   line (actor · relative time · duration). A **view toggle** to a dense **table** (columns: status ·
   trigger · repo · branch · actor · created · duration; sortable; per-page footer). Complete the filter
   bar (status · repo · search) ADR-0016 specified. Both views client-side over the existing list API.

2. **Run detail — findings & logs (Lovable disclosure + #103).** Findings as disclosure rows: collapsed
   `severity badge · title · file:line`; expanded body + a `suggestion` block + Resources. Logs get a filter
   bar (level + search + copy/download) and disclosure rows for structured lines. Keep the live stream
   and the terminal `kubectl` snippet (#99).

3. **Shell & navigation (Cursor/Linear + Raycast).** Grouped sidebar nav with hairline section
   separators; top bar gains a search affordance that opens a **⌘K command palette** (`cmdk`) for
   jump-to (run / repo / settings) and commands (e.g. trigger review, reindex). Touches every page, so
   it's its own focused PR after the two content surfaces prove the primitives.

4. **Insights (Dub) — new surface.** An **Insights** view (or Overview section): KPI cards with real
   hierarchy (total runs, pass rate, p50 duration, active) each a drill-through into the filtered runs
   table; one runs-over-time SVG series with a date-range control; breakdown cards (by repo, by outcome)
   with inline bars. Client-side aggregation MVP; control-plane endpoint flagged as follow-up.

5. **Settings (Cursor) & Repositories (cards).** Settings → grouped `SettingsSection`s: *GitHub App*,
   *Access* (permissions/RBAC, once [#93](0023-db-backed-rbac.md) lands — a no-op placeholder until the
   claim is live), *Indexing*; honest empty controls rather than hidden ones. Repositories → cards
   showing `RepoIndex` status, last-indexed time, and the relevant action.

**Sequencing (each its own governance-compliant PR):** (1) **Runs list** timeline+table+filters
[first, per decision] → (2) run-detail findings+logs → (3) shell & nav + ⌘K → (4) Insights →
(5) Settings + Repositories. Front-loads the highest-traffic surface; defers the chrome-wide ⌘K change
until the primitives are proven; lands Insights before the lower-risk Settings/Repos cleanup.

### Consequences

- Good: every surface gets a concrete, grounded pattern; the #103 format finally reads well; the runs
  list scales with history (and offers a power-user table); the shell gains keyboard navigation; an
  insights layer answers "how's it going?" without leaving for Grafana — all within the token contract,
  so light/dark + accessibility come for free.
- Good: incremental PRs keep blast radius small and each step revertible.
- Neutral: adds two small, on-ecosystem deps (`cmdk`; charts stay hand-rolled — no chart lib) and more
  shared primitives to own (the shadcn-in-repo model, [ADR-0015](0015-web-console-design-language.md)).
- Bad: this is real surface area across five PRs and introduces an insights data path; it competes for
  time with the gated RBAC cutover ([#93](0023-db-backed-rbac.md)) and with Grafana's existing role as
  the deep observability home. Mitigated by sequencing, by client-side aggregation first (no backend
  blocker), and by scoping Insights to operator-glance metrics, not replacing Grafana.

### Follow-ups (flagged, not in this ADR's PRs)

- Control-plane **aggregation endpoint** for Insights once fetch-all-and-aggregate stops scaling.
- Server-side **sort/filter/paginate** for the runs table at the same threshold.
- Wire the *Access* settings section to real permissions once [#93](0023-db-backed-rbac.md)'s claim ships.

### References

- Screens (Refero): Cursor `04df139c-…`, Lovable `f9ee3054-…`, Doppler `1d2903be-…`, Appwrite
  `b966c079-…`, Dub `ca73bf7d-…`, Raycast `370ddbce-…`. Style north star: Linear Changelog `11d3e58a-…`.
- Builds on [ADR-0015](0015-web-console-design-language.md) (design language, unchanged) and
  [ADR-0016](0016-dashboard-information-architecture.md) (IA, extended). Finding format: #103. Data
  model + API: `services/control-plane/src/types.rs`, `apps/web/lib/api.ts`.
