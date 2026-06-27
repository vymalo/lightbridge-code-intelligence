# Jobs and task lifecycle

How work flows through Lightbridge: what triggers a task, the two job kinds, the states a task moves
through, and how cancellation + data purge work. Diagrams are Mermaid (rendered by GitHub).

> Source of truth in code: `services/control-plane/src/{webhook,admin,dispatcher,reaper,lifecycle}.rs`
> and `services/agent-runner/src/main.rs`. ADR-0004 (one Job per task), RFC-0001 (queue + dispatcher
> + reaper), ADR-0023 (permission-based authz).

## The two job kinds

Every unit of work is a **task** the control plane records, the dispatcher claims, and a dedicated
**Kubernetes Job** executes (one Job per task, ADR-0004). There are two kinds, distinguished by the
task's `command` + `target_type`:

| Job | `command` | `target_type` | Triggered by |
|---|---|---|---|
| **Index** | `index` | `repository` | A repository is **approved** by an admin (`enqueue_index_on_approve`), **or a push to the default branch** (e.g. a merged PR — `handle_push`, requires the App's `push` subscription) keeps the base index fresh. Indexes the default branch; deduped against an in-flight index. |
| **Review** | `review` | `pull_request` | A PR is **opened** (the automatic first review), or a PR comment **`@<app-handle> …`** requests a re-review. |

Other lifecycle events don't create tasks: PR `synchronize`/`reopened` do nothing (ask for a re-review
with an `@`-mention); PR `closed` **cancels** the PR's active tasks; repo removed/denied **purges** its
data (see below).

> ⚠️ **Both kinds run the same pipeline.** The agent runner's `run()` always does
> clone → semantic index (pgvector) → structural index (Neo4j) → *then* review. A **review job
> re-indexes the whole repo from scratch before reviewing** — the only thing an `index` job does
> differently is skip the final review step. This is why a review takes roughly as long as an index
> every time. See [Known inefficiency](#known-inefficiency-review-re-indexes-every-time).

## End-to-end flow

```mermaid
flowchart TD
    subgraph GitHub
        PR[PR opened / @mention comment]
        APV[Admin approves repo]
    end

    subgraph CP["Control plane (API)"]
        WH[/"POST /github/webhook<br/>(HMAC verify + delivery dedup)"/]
        ADM[/"POST /admin/repositories/:id/approve<br/>(perm: repo:approve)"/]
        CREATE["create_task → status = queued<br/>NOTIFY task_queued"]
    end

    subgraph DISP["Dispatcher (RFC-0001)"]
        CLAIM["claim: SELECT … FOR UPDATE SKIP LOCKED<br/>status = running, lease set"]
        LAUNCH["launch one Kubernetes Job<br/>(ADR-0004)"]
    end

    subgraph JOB["Agent runner Job (per task)"]
        CLONE[clone @ head SHA]
        SEM[semantic index → pgvector]
        GRAPH[structural index → Neo4j<br/>best-effort]
        REV{command == review?}
        REVIEW[OpenCode review over MCP tools]
        REPORT["report status:<br/>succeeded / failed / timed_out"]
    end

    subgraph STORES[Datastores]
        PGV[(pgvector<br/>code_chunks)]
        NEO[(Neo4j<br/>graph)]
    end

    WB["Control plane validates findings<br/>vs the PR diff → posts review (ADR-0022)"]

    PR --> WH --> CREATE
    APV --> ADM --> CREATE
    CREATE -->|NOTIFY / poll| CLAIM --> LAUNCH --> CLONE
    CLONE --> SEM --> GRAPH --> REV
    SEM -.writes.-> PGV
    GRAPH -.writes.-> NEO
    REV -->|yes| REVIEW --> WB
    REV -->|no, index job| REPORT
    REVIEW --> REPORT
    WB --> REPORT
```

Notes:
- The runner holds **no GitHub App key**: it fetches a short-lived installation token + repo coords
  from `GET /internal/tasks/{id}` (shared-bearer `AGENT_RUNNER_TOKEN`), and the control plane is the
  only component that writes to GitHub (ADR-0002).
- Index/search go through the control plane's `/internal/tasks/{id}/{chunks,graph,search}` endpoints —
  the Job has no datastore credentials (ADR-0020).
- The review is **validated against the PR diff** by the control plane before write-back (ADR-0022);
  findings outside the diff are dropped or deferred.

## Task state machine

```mermaid
stateDiagram-v2
    [*] --> queued: create_task (webhook / approve)
    queued --> running: dispatcher claims (lease)
    running --> succeeded: runner reports ok
    running --> failed: runner reports error
    running --> timed_out: activeDeadlineSeconds / reaper
    queued --> cancelled: PR closed / repo denied / manual cancel
    running --> cancelled: PR closed / repo denied / manual cancel
    running --> queued: lease expired, Job dead → reaper requeues (≤ max attempts)
    succeeded --> [*]
    failed --> [*]
    timed_out --> [*]
    cancelled --> [*]
```

- Tasks are created directly as **`queued`**. (`received` and `waiting_for_index` exist in the model
  for the not-yet-built scheduler gate, RFC-0001 — not used by the current create path.)
- `posting_result` is a transient sub-state the runner may report while it writes the review back.
- The **reaper** (in the dispatcher, every ~30s) reconciles: it requeues a task whose lease expired
  but whose Job is dead (up to a max-attempts cap), and it deletes the Jobs of `cancelled` tasks.

## Cancellation

A task becomes `cancelled` three ways: a PR is **closed** (`cancel_active_tasks_for_pr`), a repo is
**removed/denied** (`cancel_active_tasks_for_repo`), or a user clicks **Cancel run** (manual,
`POST /tasks/{id}/cancel`, perm `task:cancel`). The DB row flips to `cancelled` immediately; stopping
the **pod** then happens two ways, belt-and-suspenders:

```mermaid
sequenceDiagram
    participant U as PR close / deny / manual
    participant CP as Control plane (DB)
    participant RP as Reaper (dispatcher, ~30s)
    participant RN as Agent runner (pod)
    U->>CP: status = cancelled
    par Reaper backstop
        RP->>CP: list cancelled tasks with a Job
        RP->>RN: delete Job (background) → K8s SIGTERM
        RN-->>RN: SIGTERM arm → exit promptly
    and Runner self-cancel (resilient to reaper downtime)
        loop every 10s
            RN->>CP: GET /internal/tasks/{id}/status
            CP-->>RN: cancelled
        end
        RN-->>RN: exit promptly (no status report)
    end
```

The self-cancel poll matters because the reaper lives in the dispatcher; if the dispatcher is
restarting (a deploy) the reaper can't send SIGTERM, so the runner stops itself instead. The runner
never reports a status on cancellation — the control plane already owns the `cancelled` row.

## Data purge (repo removed or denied)

When a repo is removed from the installation or denied, its indexed data is purged so we don't retain
code for repos nobody opted into (`lifecycle::purge_repository_data`): it cancels in-flight tasks and
deletes the **pgvector `code_chunks`**, the **`repo_index`** bookkeeping, and the **Neo4j graph**
(`neo4j::delete_repo_graph`). The repository row is kept (`disabled`) for audit. Purge is idempotent
and guarded against a re-approve racing ahead of it.

Purge runs two ways: a **prompt** spawned task on the remove/deny event (`spawn_purge`), and a
**durable reconciler** on the dispatcher loop (`reconcile_purges`, every reap tick) that re-purges any
`disabled` repo still carrying index data — so a purge lost to a control-plane restart still completes.
It lives in the control plane (not a per-repo k8s Job) because purge writes to Postgres + Neo4j
directly, and only the control plane holds those credentials (ADR-0020/0002).

## Indexing strategy: review reuses the base index (ADR-0025)

Originally a review job re-ran the full semantic + structural index before reviewing (the pipeline
indexed unconditionally) — so a PR review cost ≈ a full repo index every time, re-embedding unchanged
files. **Fixed in [ADR-0025](adr/0025-review-reuses-base-index.md):** the control plane reports
`repo_indexed` in the task context, and the runner indexes only for an **`index`** task or a **cold
repo**. A review on an already-indexed repo **skips the re-index** and reviews against the base index
(searched via the MCP tools) + the PR diff. So in the flow diagram, the `semantic/graph index` steps
run for index jobs and cold-start reviews; warm reviews go straight to the review step.

The base index tracks the **default branch**, so it can go stale as that branch moves. This is now
kept fresh by a **push-driven re-index**: a push to the default branch (e.g. a merged PR) queues an
`index` task (`handle_push`, deduped against an in-flight index), and retrieval otherwise reuses the
latest indexed snapshot ([ADR-0050](adr/0050-retrieval-pins-to-latest-indexed-snapshot.md)). **This
requires the GitHub App to subscribe to the `push` event** — without it the base index only ever runs
once (on approval) and goes stale. Still future: incremental diff-only indexing for reviews so search
also covers brand-new PR symbols before they are merged.

## Future direction (proposed ADRs 0028–0031)

Today the runner's pipeline (clone → index → graph → review) is **hardcoded** in
`agent-runner/src/main.rs:run()`, and the Job is a single container whose shape is built in
`control-plane/src/k8s.rs:job_manifest()`. A set of **proposed** ADRs reshapes this:

- **[ADR-0028](adr/0028-agent-job-control-sidecar.md)** — restructure the Job into a small **control
  sidecar** (bootstrap, ordering, status, the self-cancel poll, log shipping) + a **single configurable
  main container** (the heavy work). The pipeline stays a **closed built-in set**, expressed as internal
  ordered config (so conditions like ADR-0025 reuse are declared, and phases are resource-sized). One
  Job per task (ADR-0004) is unchanged; the trust boundary (ADR-0002/0017/0020/0022) is preserved.
- **[ADR-0029](adr/0029-focused-review-not-generic-runner.md)** — scope boundary: Lightbridge stays a
  **focused code-review system, not a generic step/CI runner**. We explicitly reject arbitrary
  `command`/SAST steps in the Job, a `ReviewJob` CRD/operator, and external workflow engines —
  extensibility belongs at the *understanding* layer, not the *execution* path.
- **[ADR-0030](adr/0030-repo-review-config.md)** / **[ADR-0031](adr/0031-review-skills-commands.md)** —
  that understanding layer: an optional `.lightbridge-code-review.jsonc` lets a repo declare conventions,
  focus/ignore paths, instructions, and named **skills/commands** (`@mention`-invokable) that ground the
  review — read as **data, never executed**, and still validated/posted on the trusted side (ADR-0022).
