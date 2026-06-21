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
| **Index** | `index` | `repository` | A repository is **approved** by an admin (`enqueue_index_on_approve`). Indexes the default branch. |
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

> Today purge runs as a best-effort async task in the control plane (`spawn_purge`); hardening it into
> a durable, observable Job/reconciler (survives a control-plane restart) is planned.

## Known inefficiency: review re-indexes every time

As noted above, a **review job re-runs the full semantic + structural index** before it reviews,
because the runner's pipeline indexes unconditionally. Consequences:
- A PR review costs ≈ a full repo index every time (embeddings + graph rebuild), which is why the two
  take similar wall-clock.
- It re-embeds unchanged files and rebuilds the whole graph even when only a few files changed.

Planned fix: skip the re-index when the repo's `repo_index` is already fresh for the head SHA (or
index only the PR's changed files), so review work is proportional to the diff. Tracked as a follow-up
(ADR + PR).
