# ADR-0028: Agent Job = control sidecar + single configurable main container

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

A task runs as one Kubernetes Job ([ADR-0004](0004-one-k8s-job-per-task.md)). Today that Job is a
**single `runner` container** whose pipeline is **hardcoded in Rust** (`agent-runner/src/main.rs:run()`:
clone → semantic-index → structural-graph → review), and whose pod shape is assembled in
`control-plane/src/k8s.rs:job_manifest()` as a fixed container with one `resources` block. Two stable,
*operational* concerns — lifecycle/control (bootstrap per [ADR-0017](0017-agent-runner-control-plane-bootstrap.md),
status reporting, the self-cancel poll #116, log shipping) and the swappable *heavy* work
(clone/index/graph/the native review agent of [ADR-0026](0026-native-review-agent.md)) — live in the
**same process, image, and resource envelope**.

How should the per-task Job be structured so the stable control concern is separated from the heavy
work and each phase can be resourced appropriately — **without** turning the Job into a generic,
operator-extensible step runner (that boundary is decided in [ADR-0029](0029-focused-review-not-generic-runner.md))?

## Decision Drivers

- **Separation of concerns:** isolate the rarely-changing control/lifecycle surface from the heavy,
  frequently-changing work so each evolves and is tested independently.
- **Independent evolution of the heavy image:** the review agent (ADR-0026) and indexers change often;
  the control surface (bootstrap/status/cancel/logs) should not churn with them.
- **Right-sized resources** per phase instead of one envelope for clone + embed + LLM review.
- **Preserve the trust boundary** ([ADR-0002](0002-rust-control-plane-trust-boundary.md),
  ADR-0017, [ADR-0020](0020-mcp-servers-via-control-plane.md),
  [ADR-0022](0022-review-writeback-control-plane.md)): no standing creds in the Job.
- **Stay focused:** a **closed** built-in pipeline, not an open plugin system — see ADR-0029.
- **Don't break the working path.**

## Considered Options

- **A. Status quo** — single hardcoded `runner` container; pod shape fixed in Rust.
- **B. Control sidecar + single configurable main container** (chosen) — a small sidecar owns the
  lifecycle; one main container does the heavy work; the pipeline is the **closed built-in set**
  expressed as **internal ordered config** (data, not hardcoded `if`s).

> The tempting third direction — an *externally-defined, operator-extensible* pipeline with arbitrary
> `command`/SAST steps (and the CRD/operator or workflow-engine variants) — is considered and
> **rejected** in [ADR-0029](0029-focused-review-not-generic-runner.md). It is not re-litigated here;
> this ADR assumes the closed built-in set ADR-0029 settles on.

## Decision Outcome

Chosen option: **B — a control sidecar + a single configurable main container.**

- **Control sidecar** (always present; small fixed image; minimal requests, ~50m/64Mi). The *control
  plane of the pod*: it bootstraps from the control plane (`GET /internal/tasks/{id}`, ADR-0017),
  drives the built-in step sequence, enforces per-step timeouts, owns the **self-cancel poll** + SIGTERM
  (ADR-0024/#116), ships logs/metrics, and reports the **terminal status**. It holds the short-lived
  installation token and `AGENT_RUNNER_TOKEN` and hands the main container only what it needs over a
  shared `emptyDir` workspace (and/or a localhost loopback) — not via mounted Secrets.
- **Main container** (the heavy work): clone, semantic index (pgvector), structural graph (Neo4j), and
  the native review agent (ADR-0026). Right-sized resources; reads the cloned working tree (including
  the repo config of [ADR-0030](0030-repo-review-config.md)).
- **Pipeline = closed built-in set** (`clone`/`index`/`graph`/`review`) expressed as **internal ordered
  config** so the per-phase failure policy (the graph step is best-effort today) and the conditions
  (`repo_indexed` reuse per [ADR-0025](0025-review-reuses-base-index.md); "review only for PRs") become
  declared **data** instead of hardcoded branches. **Operator-tunable knobs** — which built-ins run,
  per-phase resources, deadlines — stay in the existing control-plane config (`config.rs` `agent.*`).
  This is *configuration of a fixed set*, **not** a plugin system (ADR-0029).

### Migration (phased — don't break the working path)

1. Refactor `main.rs:run()` so the current stages are an internal ordered list with
   conditions/failure-policy as data — no behavior change, single container.
2. Introduce the sidecar; move bootstrap + status + self-cancel poll + log shipping into it; the heavy
   work stays in the main container; they share an `emptyDir`. Behind `JOB_LAYOUT=sidecar|single`,
   default `single`.
3. Sidecar drives the built-in steps; reach parity; flip the default to `sidecar`.

### Consequences

- **Good:** the stable control surface lives in one small, rarely-changing sidecar image; the heavy
  image (ADR-0026 agent, indexers) evolves and is resourced independently; per-phase resource sizing;
  lifecycle logic (bootstrap/status/cancel/logs) in one place.
- **Bad:** multi-container Jobs add complexity — log streaming (#88) must select the right container,
  the reaper's "done" view spans containers, and there's intra-pod coordination over the shared
  workspace.
- **Neutral:** ADR-0004 (one Job per task) is unchanged; arbitrary steps / CRD / workflow engines are
  explicitly out of scope (ADR-0029); product extensibility lives at the *understanding* layer
  ([ADR-0030](0030-repo-review-config.md)/[ADR-0031](0031-review-skills-commands.md)), not here.

## Pros and Cons of the Options

### A. Status quo (single hardcoded container)

- Good: simplest; one image/process; easy logs; smallest surface.
- Bad: control + heavy work entangled in one image and one resource envelope; conditions are hardcoded
  branches.

### B. Sidecar + single configurable main (chosen)

- Good: clean control/heavy split; independent evolution + sizing; conditions as data; keeps ADR-0004
  and the trust boundary; stays focused (closed set).
- Bad: multi-container complexity (log/reaper/coordination).

## More Information

- Restructures the interior of the Job in [ADR-0004](0004-one-k8s-job-per-task.md) (one Job per task
  unchanged). The sidecar becomes the bootstrapper of
  [ADR-0017](0017-agent-runner-control-plane-bootstrap.md); the trust-boundary properties of
  [ADR-0002](0002-rust-control-plane-trust-boundary.md),
  [ADR-0020](0020-mcp-servers-via-control-plane.md), and
  [ADR-0022](0022-review-writeback-control-plane.md) are preserved.
- Scope boundary that shapes this (closed built-in set; reject generic steps/CRD/engine):
  [ADR-0029](0029-focused-review-not-generic-runner.md).
- The native review agent ([ADR-0026](0026-native-review-agent.md)) and base-index reuse
  ([ADR-0025](0025-review-reuses-base-index.md)) are a built-in step and a declared condition.
- Current implementation this would refactor: `control-plane/src/k8s.rs` (`job_manifest()`),
  `control-plane/src/config.rs` (`agent.*`), `agent-runner/src/main.rs` (`run()`). Context:
  `docs/jobs-and-lifecycle.md`, `docs/kubernetes-deployment.md`.
