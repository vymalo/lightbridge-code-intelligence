# Requests for Comments (RFCs)

RFCs are how we propose and socialize **substantial** changes to Lightbridge Code Intelligence
before a decision is made. They give a structured space for motivation, design detail, drawbacks,
and alternatives — the deliberation that a terse [ADR](../adr/README.md) does not capture. The model
is the [Rust RFC process](https://github.com/rust-lang/rfcs).

## When to write an RFC

Write an RFC when a change is large or far-reaching enough that it benefits from review *before*
implementation. Examples:

- a new subsystem or a cross-cutting architectural change
- a change to a public contract (API, schema, MCP tool surface)
- a security-sensitive design
- anything where reasonable engineers would want to weigh alternatives first

Small, obvious, or easily reversible decisions can skip the RFC and go straight to an
[ADR](../adr/README.md).

## Lifecycle

```
Draft ──▶ Proposed ──▶ Accepted ──▶ (yields one or more ADRs)
                   └─▶ Rejected
```

- **Draft** — author is still writing; not yet ready for wide review.
- **Proposed** — open for review and discussion.
- **Accepted** — agreed; the author records the resulting decision(s) as one or more
  [ADRs](../adr/README.md) and links them here.
- **Rejected** — not adopted; kept for the historical record and to avoid relitigating.

## Relationship to ADRs

An RFC is the *proposal and discussion*; an ADR is the *recorded, immutable decision*. An accepted
RFC typically yields one or more ADRs. See [ADR-0012](../adr/0012-rfc-process-alongside-adrs.md).

## Authoring

Copy [the template](0000-rfc-template.md) to `NNNN-kebab-title.md`, numbered sequentially.

## Index

| # | Title | Status |
|---|---|---|
| [0000](0000-rfc-template.md) | RFC template | — |

_No substantive RFCs yet. New proposals are added here._
