# ADR-0029: Scope boundary — a focused code-review system, not a generic step/CI runner

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

While shaping the per-task Job restructure ([ADR-0028](0028-agent-job-control-sidecar.md)) we considered
making the pipeline **operator-extensible**: arbitrary `command`/step images (e.g. SAST, license/secret
scans) added by a site administrator, with the job shape promoted to a `ReviewJob` **CRD + operator**,
or delegated to an external **workflow engine** (Argo Workflows / Tekton). It's tempting — "we're
already almost an operator, why not?"

This ADR records the deliberate decision **not** to go there, and the reasoning, so future work doesn't
drift back into it. The positive corollary — where extensibility *should* live — is the point.

## Decision Drivers

- **Mission focus.** The system's value is the core: `(neo4j + pgvector) × (graph + embedding) →
  review`. Step orchestration is not the product.
- **Security surface.** The Job processes **untrusted repo content** (including forks).
- **Long-term maintenance.** Every general-purpose mechanism is owned forever.
- **The built-in step set is small and stable** (clone/index/graph/review) — the leverage of a generic
  runner isn't there.

## Decision Outcome

**Lightbridge's per-task Job runs a closed, built-in pipeline (clone/index/graph/review). We reject
turning it into a generic step/CI runner.** Specifically we reject:

1. **Arbitrary operator-defined `command`/step images in the Job** (e.g. running SAST inside Lightbridge).
   - **Denatures the mission:** it makes a focused reviewer into a generic CI runner; the value is the
     retrieval-grounded review, not orchestrating steps.
   - **Security:** arbitrary images executing over untrusted repo content is a large new surface;
     egress and secret containment become a permanent battle.
   - **Maintenance:** a step schema, image lifecycle, and failure semantics to own forever — for steps
     that would be tiny next to the indexing/review core.
2. **A `ReviewJob` CRD + operator.**
   - Buys declarative management and `kubectl get reviewjobs` for a *small, fixed* step set — near-zero
     product value against the cost of a reconciler, CRD versioning, RBAC, and a second control loop.
     "Almost an operator" is not a reason to become one.
3. **An external workflow engine (Argo Workflows / Tekton).**
   - A heavy platform dependency and operational surface, and an awkward fit with our per-task bootstrap
     and trust model ([ADR-0017](0017-agent-runner-control-plane-bootstrap.md)). Over-scoped.

### The principle: extend understanding, not execution

Customization belongs at the **input / context layer** — the reviewed repo telling Lightbridge *how to
read it* — consumed as **data, never executed**:

- [ADR-0030](0030-repo-review-config.md): a repo-level review config (`.lightbridge-code-review.jsonc`).
- [ADR-0031](0031-review-skills-commands.md): custom review skills/commands defined in that config.

This keeps flexibility where it serves the mission (better, repo-aware reviews) and out of the
execution path where it would add risk and maintenance.

### Consequences

- **Good:** the system stays small and focused; minimal attack surface; low long-term maintenance; a
  clear, citable "no" for feature-creep requests; a stated principle to evaluate future asks against.
- **Bad:** a team's own tooling (e.g. their SAST) is **not** run inside Lightbridge — that stays in
  their CI. Lightbridge could *consume* such results as context via the repo config, but never executes
  them. Some users may want a one-stop pipeline.
- **Neutral:** this is a living boundary — if the mission itself changes, a future ADR can supersede it.

## Pros and Cons of the Options

### Closed built-in pipeline (chosen)

- Good: focused, secure, low-maintenance; extensibility redirected to the safe understanding layer.
- Bad: no in-Job arbitrary tooling.

### Generic step runner / CRD / workflow engine (rejected)

- Good: maximal flexibility; one-stop pipeline; declarative management.
- Bad: denatures the mission; large security + maintenance surface for marginal value; over-scoped.

## More Information

- Shapes [ADR-0028](0028-agent-job-control-sidecar.md) (closed built-in pipeline) and enables the
  understanding-layer extensibility of [ADR-0030](0030-repo-review-config.md) and
  [ADR-0031](0031-review-skills-commands.md).
- Related: [ADR-0004](0004-one-k8s-job-per-task.md) (one Job per task),
  [ADR-0026](0026-native-review-agent.md) (the review agent that the closed pipeline runs).
