# ADR-0016: Dashboard information architecture & task-run views

- **Status:** Accepted
- **Date:** 2026-06-19

## Context and Problem Statement

The console must "display each task run and everything else." A **task** is a unit of work the
control plane owns (triggered by a GitHub event — PR, push, or comment) that runs the
index→review/Q&A pipeline; see the `Task`/`TaskStatus` model in
`services/control-plane/src/types.rs`. We need an information architecture and the concrete
list/detail patterns before building, grounded in real consoles (Refero research).

Primary references: **Vercel deployments** list (`vercel.com/.../deployments`) for the run list;
**Appwrite** deployment detail + logs (incl. a failed state) for run detail; **Cursor** for the
dark console shell; **Linear** for status treatment; **Zendesk** for the empty state.

## Decision Drivers

- Status-first scannability: see what's running/failed across repos at a glance.
- Drill-down from a run to its **output** (structured review payload) and **logs**.
- Honest states: today the dashboard is empty — first-run, loading, and error states matter.
- Map cleanly to the existing `Task`/`RepoIndex` models (no invented data).

## Decision Outcome

**Shell (Cursor/Vercel):** left sidebar nav — Overview · Repositories · Runs · Settings — + a slim
top bar (org/user, global search). Contained max-width main content.

**Dashboard = task-run list (Vercel deployments pattern):** stacked rows, each:
`status pill · title (trigger: "PR #123" / commit / "@bot review") · repo · branch · actor ·
relative time · duration`. A filter bar (status, repo, branch) + search above it. Rows link to the
run detail.

**Run detail (Appwrite pattern):** an **overview card** (status, repo, trigger, base/head SHA,
queued/started/finished timings, duration) above sectioned, collapsible panels for the **structured
review output** (findings with line refs) and **logs**. Failed runs surface the error prominently.

**Status model** — map `TaskStatus` to a small visual set ([ADR-0015](0015-web-console-design-language.md) tokens):
- `Received`, `WaitingForIndex`, `Queued` → **pending** (gray)
- `Running`, `PostingResult` → **active** (blue, subtle motion)
- `Succeeded` → **success** (green) · `Failed`, `TimedOut` → **error** (red) · `Cancelled` → **muted**

**Repositories view:** connected repos + their `RepoIndex` status (`Pending/Running/Ready/Failed/
Stale/Disabled`) so users see indexing health.

**States:**
- **First-run empty** (no runs yet — today's case): a centered prompt with a one-line explainer +
  primary action (connect a repo / install the GitHub App). Zendesk-style; this is the one place a
  centered placard is right (the screen has nothing else to do).
- **Per-section empties / loading / errors:** inline status lines and row skeletons, not placards
  (house rule).

### Consequences

- Good: a concrete, reference-grounded blueprint the implementation can follow; maps 1:1 to the
  data model; defines the states we currently lack.
- Neutral: depends on real API data (run list/detail), which lands with persistence + the GitHub
  App; until then views render against fixtures.
- Bad: list/detail + states is real surface area — scope the first cut to the run list + detail and
  defer charts/analytics.

### References

- Screens (Refero): Vercel deployments `8c510eb3…` / `a1f3af4e…`; Appwrite detail+logs `8d9565bd…`
  (failed) / `91b6d366…`; Cursor dashboard `7573ea52…`; Linear statuses `26ec3bd8…`; Zendesk empty
  state `169f3668…`.
- Design language: [ADR-0015](0015-web-console-design-language.md). Data model:
  `services/control-plane/src/types.rs`, `schema/control-plane.cstack`.
