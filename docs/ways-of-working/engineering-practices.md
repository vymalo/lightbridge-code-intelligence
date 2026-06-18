# Engineering Practices

How this team works. The delivery model is **XP + Lean + DevOps with shift-left**, operating under
the ADORSYS-GIS AI Governance doctrine. This is recorded as
[ADR-0011](../adr/0011-engineering-practices-xp-lean-devops.md).

## Extreme Programming (XP)

- **Pairing** — non-trivial and security-sensitive work is paired (or mob-reviewed). Two sets of
  eyes on the trust boundary, the auth path, and the indexing pipeline.
- **Test-Driven Development** — write the failing test first, then the code. The control plane's
  webhook handling, idempotency, line-validation, and the `/auth/verify` contract are natural TDD
  targets.
- **Continuous Integration** — `main` stays green. Every change runs the quality gates in CI
  (mirroring the local pre-push gates below).
- **Small releases** — ship small, frequent, reversible changes. Prefer many small PRs over a big
  one.
- **Collective ownership** — anyone may improve any part of the codebase. There are no permanent
  silos; the docs, ADRs, and shared tooling exist so anyone can pick up any area.

## Lean software development

- **Eliminate waste** — no speculative abstractions, no gold-plating, no work that doesn't move a
  task to Done.
- **Build quality in** — quality is designed and tested in, not inspected in at the end (see
  shift-left below).
- **Deliver fast** — short cycle times; keep WIP low so work flows.
- **Amplify learning** — RFCs and ADRs capture decisions and their context so the team learns once
  and doesn't relitigate. Spikes are time-boxed.

## DevOps

- **You build it, you run it** — the team that ships a service operates it. Ownership doesn't end
  at merge.
- **Automation** — `just` is the single human-facing entrypoint; heavier Rust automation lives in
  `cargo xtask`. Manual steps are automated away.
- **Observability** — structured logs, metrics, and traces are part of the work, not an
  afterthought (see [security, observability, testing, rollout](../security-observability-testing-rollout.md)).

## Shift-left

Testing, security, and review happen **early**, not at the end. Concretely, run the quality gates
locally **before pushing**:

```bash
just lint   # pnpm lint + cargo clippy --all-targets -- -D warnings
just test   # pnpm test + cargo nextest run
just fmt    # biome format + rustfmt
```

These tie directly to the tooling ([ADR-0013](../adr/0013-local-dev-and-build-tooling.md)):

- **cargo-nextest** — fast Rust test runner; the unit/integration layer.
- **wiremock** — mock outbound HTTP (e.g. GitHub) in Rust tests, so contracts are tested without
  the network.
- **clippy** — Rust linting at `-D warnings`; warnings are errors.
- **Biome** — JS/TS formatting and linting for `apps/web` and `packages/*`.
- **The governance PR gate** — the AI Governance caller workflow enforces checks on every PR.

Security shifts left too: least-privilege design, secret redaction, and prompt-injection fixtures
are written alongside the feature, not bolted on later.

## Connection to AI Governance

The ADORSYS-GIS AI Governance framework
([ADR-0008](../adr/0008-adopt-ai-governance-framework.md)) gives us a shared contract for *when*
work is ready and done:

- **Definition of Ready (DoR)** — a task is only started when it is well-formed: clear acceptance
  criteria, known scope, and (where relevant) a declared approach.
- **Definition of Done (DoD)** — a change is only Done when it is tested, the quality gates pass,
  observability is in place, docs/ADRs are updated, and the governance gate is green.
- **AI usage declarations** — contributors declare where and how AI assisted a change, so reviews
  account for it. This is consistent with collective ownership: the human author remains
  accountable.

## Related decisions

- [ADR-0008: AI Governance framework](../adr/0008-adopt-ai-governance-framework.md)
- [ADR-0011: Engineering practices](../adr/0011-engineering-practices-xp-lean-devops.md)
- [ADR-0012: RFC process](../adr/0012-rfc-process-alongside-adrs.md)
- [ADR-0013: Local dev & build tooling](../adr/0013-local-dev-and-build-tooling.md)
