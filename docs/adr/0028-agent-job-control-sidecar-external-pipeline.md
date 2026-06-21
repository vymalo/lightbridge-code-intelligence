# ADR-0028: Agent Job as a control sidecar + heavy main container, driven by an externally-defined pipeline

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

A task runs as one Kubernetes Job ([ADR-0004](0004-one-k8s-job-per-task.md)). Today that Job is a
**single `runner` container** whose pipeline is **hardcoded in Rust**: `agent-runner/src/main.rs:run()`
clones → semantic-indexes (pgvector) → structural-graphs (Neo4j, best-effort) → reviews, with the
sequence, the per-stage failure policy, and the stage-selection logic (index task vs review task,
`repo_indexed` reuse per [ADR-0025](0025-review-reuses-base-index.md)) all baked into the binary. The
Job manifest itself is assembled in `control-plane/src/k8s.rs:job_manifest()` — a fixed container list
with one `resources` block, two optional volumes (internal-CA, agent-config), and a fixed env set.

This is rigid in three ways:

- **You cannot add a step without shipping code.** A natural operator ask — "also run a SAST scan,"
  "add a license check," "mount a tool cache volume" — requires editing the runner and/or
  `job_manifest()` and redeploying. There is no seam for site-specific steps.
- **One resource envelope for very different work.** Clone, embedding-index, and LLM review have
  wildly different CPU/memory/time profiles, but the whole runner shares one `agent.resources` block.
- **Control logic and heavy lifting are entangled.** Status reporting, the self-cancel poll
  ([ADR-0024](0024-web-console-redesign-v2.md)/#116), SIGTERM handling, and log/metric shipping live
  in the same process and image as the heavy clone/index/review work, so the stable "lifecycle"
  concern rides every change to the heavy concern (and vice-versa).

How should the per-task Job be structured so that **a site administrator can compose the pipeline
(add steps, volumes, resources) without code changes**, while preserving the trust-boundary
guarantees the current design depends on ([ADR-0002](0002-rust-control-plane-trust-boundary.md),
[ADR-0017](0017-agent-runner-control-plane-bootstrap.md), [ADR-0020](0020-mcp-servers-via-control-plane.md))?

## Decision Drivers

- **Extensibility without forking the code.** Adding a step (SAST, license/secret scan, a custom
  command) or a volume should be a configuration change an operator owns, not a runner/control-plane
  code change + image rebuild.
- **Separation of concerns.** A small, stable *control* surface (bootstrap, ordering, status,
  cancellation, log shipping) distinct from the *heavy* work (clone/index/graph/review/scan), so each
  evolves and is resourced independently.
- **Right-sized resources** per step instead of one envelope for the whole run.
- **Preserve the trust boundary.** No standing credentials in the Job; the control plane stays the
  only holder of the GitHub App key and datastore creds; untrusted repo content (incl. forks) stays
  contained; operator-added steps must not silently gain cluster authority.
- **Don't break the working path.** The current single-container pipeline must keep running while this
  lands incrementally.
- **One decision, one Job per task.** ADR-0004's isolation/cleanup model stays; this changes the Job's
  *internal shape*, not the one-Job-per-task contract.

## Considered Options

- **A. Status quo** — hardcoded stages in a single runner container; job shape built in Rust.
- **B. Control sidecar + heavy main container(s), driven by an externally-defined pipeline spec**
  (chosen). The Job always carries a small **control sidecar**; the **heavy work** runs in the main
  container(s); the **ordered list of steps** (and their images/resources/volumes/conditions/failure
  policy) is a **declarative document supplied by the operator** (a mounted ConfigMap now, shaped so it
  can back a CRD later).
- **C. A full Lightbridge operator + `ReviewJob` CRD.** The most declarative/Kubernetes-native option,
  but a large build-and-operate surface (a reconciler, CRD versioning, RBAC, an extra control loop).
- **D. Adopt an existing workflow engine (Argo Workflows / Tekton).** Powerful step DAGs for free, but
  a heavy new platform dependency and operational surface, and an awkward fit for our per-task
  bootstrap/trust model — over-scoped for the current need.

## Decision Outcome

Chosen option: **B — a control sidecar + heavy main container(s), driven by an externally-defined
pipeline spec.** It directly answers the "compose the pipeline without code" driver with the least new
platform surface, and it cleanly separates the stable control concern from the heavy work while keeping
ADR-0004's one-Job-per-task model. Option C (CRD/operator) is the natural *evolution* of B — B's
pipeline document is deliberately designed to become a CRD spec — so we get most of the value now and
keep the door open. Option D is rejected as too heavy for the need.

### Shape

The dispatcher still builds **one Job per task** (ADR-0004), but `job_manifest()` assembles the pod
from a **pipeline spec** + defaults instead of a fixed container list:

- **Control sidecar** (always present; small fixed image; minimal requests, e.g. ~50m / 64Mi). It is
  the *control plane of the pod*: it bootstraps from the control plane
  (`GET /internal/tasks/{id}`, ADR-0017), reads the pipeline spec, **drives the steps in order**,
  enforces per-step timeouts, owns the **self-cancel poll** + SIGTERM handling (ADR-0024/#116), ships
  logs/metrics, and reports the **terminal status** (`POST /internal/tasks/{id}/status`). It holds the
  short-lived installation token and the `AGENT_RUNNER_TOKEN`; it hands steps only what they need over
  a **shared `emptyDir` workspace** (and/or a localhost loopback) — not via mounted Secrets.
- **Heavy main container(s) / steps** do the actual work (clone, index, graph, the native review agent
  of [ADR-0026](0026-native-review-agent.md), and operator-added steps like SAST). Each step is
  right-sized and shares the workspace volume. A step's *type* is implemented in code (the built-in
  clone/index/graph/review remain Rust); a step's *presence, order, image, resources, env, volumes,
  condition, and failure policy* are **data** in the pipeline spec.

### The pipeline spec (externally defined)

A declarative document the operator owns — initially a **ConfigMap** mounted into the pod (and read by
the dispatcher when composing the manifest), versioned with a `apiVersion`-like field so it can later
become a CRD. Sketch:

```yaml
pipeline:
  steps:
    - name: clone
      type: builtin                 # clone | index | graph | review (Rust-implemented step types)
      fatal: true
    - name: index
      type: builtin
      when: "!repo_indexed"         # condition over the bootstrap context (ADR-0025 reuse)
      resources: { requests: { cpu: "1", memory: "2Gi" } }
    - name: graph
      type: builtin
      fatal: false                  # best-effort, matches today's non-fatal graph step
    - name: sast                    # <-- operator-added, no code change
      type: command
      image: ghcr.io/acme/semgrep:1.x
      command: ["semgrep", "--config", "auto", "--json", "-o", "/workspace/sast.json"]
      volumes: [{ name: tool-cache, mountPath: /cache }]
      fatal: false
    - name: review
      type: builtin
      when: "target_type == 'pull_request'"
  volumes:
    - name: tool-cache
      persistentVolumeClaim: { claimName: lightbridge-tool-cache }
```

- **`builtin` steps** map to the existing Rust stages; the spec controls *whether/when* they run and
  with what resources — replacing the hardcoded sequence and the in-code stage selection.
- **`command` steps** run an operator-chosen image against the shared workspace — the SAST/license/secret-scan
  extension point. Their output is left in the workspace for a later step (or the sidecar) to collect.
- **`when`** conditions evaluate against the bootstrap context (`command`, `target_type`,
  `repo_indexed`, …), so `repo_indexed` reuse (ADR-0025) and "review only for PRs" become declared
  conditions rather than `if` branches.
- **`fatal`** captures the per-step failure policy already implicit today (the graph step is
  best-effort/non-fatal).

### Migration (phased — don't break the working path)

1. **Extract the implicit pipeline.** Refactor `main.rs:run()` so the current stages are an *internal*
   ordered list with conditions/failure-policy data — no behavior change, single container.
2. **Introduce the sidecar.** Move bootstrap + status + self-cancel poll + log shipping into a small
   control binary; the heavy work stays in the main container; they share an `emptyDir`. Behind a flag
   (e.g. `JOB_LAYOUT=sidecar|single`), default `single`.
3. **Spec-drive the built-ins.** Have the dispatcher read the pipeline ConfigMap and compose the
   manifest from it; the sidecar executes the declared `builtin` steps. Reach parity with today.
4. **Open the `command` step type** (operator-added steps + volumes) and flip the default to `sidecar`.
5. **(Later, optional)** promote the pipeline document to a `ReviewJob` CRD with a small reconciler
   (Option C) if declarative management / status surfacing warrants it.

### Consequences

- **Good:** operators compose the pipeline — add a SAST/license/secret-scan step, mount a cache or
  output volume, retune per-step resources — **without touching code or redeploying** the control
  plane or runner.
- **Good:** the stable control concern (bootstrap, ordering, cancellation, status, log shipping) lives
  in one small, rarely-changing sidecar image; the heavy image can change and be resourced
  independently. The native review agent (ADR-0026) is just one `builtin` step.
- **Good:** per-step resources replace the one-size envelope; the sidecar's footprint is tiny.
- **Good:** it's the stepping stone to a CRD/operator (Option C) without committing to that surface now.
- **Bad:** more moving parts per pod (≥2 containers, a shared volume, intra-pod coordination) and a new
  **pipeline-spec schema** to design, validate, and version. Multi-container Jobs complicate log
  streaming (#88 reads pod logs directly — it must now select the right container) and the reaper's
  view of "done."
- **Bad / risk:** `command` steps run operator-chosen images over **untrusted repo content**. They
  must run under the same least-privilege ServiceAccount, receive **no extra Secrets or cluster
  credentials by default**, and not be handed the control-plane bearer / installation token unless
  explicitly granted. Adding a step is a security decision; the spec and docs must make that explicit
  (and a `NetworkPolicy`/egress posture for step containers is a follow-up). This preserves
  ADR-0002/0017/0020/0022: the trust boundary, the no-App-key/no-datastore-creds-in-Job properties, and
  control-plane-side write-back stay intact.
- **Neutral:** ADR-0004 (one Job per task) is unchanged — this restructures the Job's interior. The
  detailed pipeline-spec schema and the trust model for `command` steps are substantial enough to
  warrant an **RFC** ([ADR-0012](0012-rfc-process-alongside-adrs.md)) before implementation.

## Pros and Cons of the Options

### A. Status quo (hardcoded single container)

- Good: simplest; one image, one process, easy logs; least surface.
- Bad: no extension seam — every step/volume/resource change is code + redeploy; one resource envelope;
  control and heavy work entangled. Fails the core driver.

### B. Control sidecar + heavy main + external pipeline (chosen)

- Good: operator-composable pipeline without code; control/heavy separation; per-step resources;
  evolvable toward a CRD; keeps ADR-0004 and the trust boundary.
- Bad: multi-container complexity; a schema to own; `command` steps widen the security surface and need
  explicit guardrails.

### C. Full operator + `ReviewJob` CRD

- Good: the most Kubernetes-native, declarative, status-rich option; `kubectl get reviewjobs`.
- Bad: large build-and-operate cost (reconciler, CRD versioning, RBAC, another control loop) for value
  we can mostly get from B now. Deferred as B's future evolution.

### D. Argo Workflows / Tekton

- Good: mature step/DAG engine, retries, artifacts for free.
- Bad: heavy new platform dependency + operational surface; awkward fit with our per-task bootstrap and
  trust model; over-scoped for the current need.

## More Information

- Amends the *shape* of the Job in [ADR-0004](0004-one-k8s-job-per-task.md) (one Job per task is
  unchanged). The sidecar becomes the bootstrapper of
  [ADR-0017](0017-agent-runner-control-plane-bootstrap.md); the trust-boundary properties of
  [ADR-0002](0002-rust-control-plane-trust-boundary.md),
  [ADR-0020](0020-mcp-servers-via-control-plane.md), and
  [ADR-0022](0022-review-writeback-control-plane.md) are preserved.
- The native review agent ([ADR-0026](0026-native-review-agent.md)) and base-index reuse
  ([ADR-0025](0025-review-reuses-base-index.md)) become a `builtin` step and a `when` condition.
- Current implementation this would refactor: `control-plane/src/k8s.rs` (`job_manifest()`),
  `control-plane/src/config.rs` (`agent.*` config), `agent-runner/src/main.rs` (`run()` stage
  sequence). Lifecycle/log context: `docs/jobs-and-lifecycle.md`, `docs/kubernetes-deployment.md`.
- Given its size, expect a follow-up **RFC** to nail the pipeline-spec schema, the `when` expression
  grammar, and the `command`-step security model before implementation.
