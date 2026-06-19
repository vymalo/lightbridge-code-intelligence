# RFC-0001: Horizontally scalable control plane (stateless roles + Postgres-backed queue)

- **Status:** Proposed
- **Author(s):** Stephane Segning Lambou
- **Date:** 2026-06-19
- **Resulting ADRs:** (filled in once accepted — anticipated: an ADR for the stateless-role split,
  and an ADR for the Postgres-backed work queue)

## Summary

The Rust control plane currently cannot run more than one replica, while the web app already runs
several. This RFC proposes making the control plane horizontally scalable by removing its only
piece of cross-request in-memory state, splitting the binary into three deployable roles, and
distributing work through a **Postgres-backed queue** over the existing `tasks` table — no new
message broker. Redis is added only as a cache/lock/rate-limit layer, off the correctness path.

## Motivation

In production the `web` app runs with multiple replicas but the control plane runs with exactly
one. The reason is not a deliberate capacity choice — it is a correctness constraint in the code.

GitHub webhook deduplication is held in process memory:

```rust
// services/control-plane/src/main.rs
/// In-memory delivery-id dedup set. Production replaces this with the Postgres
/// `github_deliveries` table (see docs/components-and-data-models.md).
pub seen_deliveries: Arc<Mutex<HashSet<String>>>,
```

```rust
// services/control-plane/src/webhook.rs
let mut seen = state.seen_deliveries.lock().expect("dedup lock poisoned");
if !seen.insert(delivery_id.clone()) {
    return (StatusCode::ACCEPTED, "duplicate delivery");
}
```

Each replica has its own `HashSet`. GitHub delivers webhooks **at least once** and retries on
non-2xx or timeout. With two replicas behind a load balancer:

- delivery `abc` → replica A → recorded in A's set → task created
- retry of `abc` → replica B → not in B's set → **duplicate task created**

That violates the idempotency contract in
[github-app-and-control-plane.md](../github-app-and-control-plane.md#idempotency-model) ("if a
duplicate delivery ID already exists, return 202 and do nothing"). So a second replica today is not
merely redundant — it is actively incorrect for the webhook path.

Notably, nothing else in the service is a scaling blocker: there is no background scheduler loop, no
startup-migration singleton, no local disk or embedded database, no in-memory connection registry,
and the JWT validator already uses a shared, self-refreshing cache. The dedup set is the **single**
thing pinning the service to one replica.

The expected outcome of this RFC: the control plane scales like the web app (N replicas for
availability and throughput), webhook handling is correct under retries and concurrency, and the
path to async/queued processing is in place without standing up a message broker.

## Guide-level explanation

We separate three concerns that the current single process conflates, and we run them as three
**roles of the same binary**, selected by a subcommand. Same image, three Kubernetes Deployments.

| Role (subcommand) | Replicas | Responsibility |
|---|---|---|
| `serve` (api) | N (stateless) | Webhook ingress + REST + health. Verifies, dedups, enqueues. Returns `202` fast. |
| `dispatcher` | 1..N | Consumes queued `tasks` and creates **one Kubernetes Job per task** ([ADR-0004](../adr/0004-one-k8s-job-per-task.md)). Tracks Job lifecycle, enforces concurrency limits. |
| `scheduler` | 1 (singleton) | Periodic GitHub pulls (installation/repo sync, reindex triggers) and the reaper that recovers stuck tasks / orphaned Jobs. |

Two important reconciliations with existing decisions:

1. **The dispatcher is not a compute worker.** Per [ADR-0004](../adr/0004-one-k8s-job-per-task.md),
   heavy/untrusted work runs in an ephemeral, per-task Kubernetes Job (indexer Job, agent Job). The
   dispatcher only *translates a queued task into a Job* and follows it; it does no repository
   reasoning itself. This keeps the per-task isolation, TTL cleanup, and per-task credentials that
   ADR-0004 requires.
2. **The queue is the `tasks` table.** The
   [task lifecycle](../github-app-and-control-plane.md#task-lifecycle) already models a queue
   (`Received → WaitingForIndex → Queued → Running → …`). We make that explicit with a
   `SELECT … FOR UPDATE SKIP LOCKED` dequeue rather than introducing RabbitMQ/Kafka.

The two flows the design must support:

- **Webhook → queue → dispatch.** `serve` verifies the HMAC signature, records the delivery in
  Postgres (durable dedup), creates the task row, and returns `202`. The `dispatcher` later picks
  the task up and spawns its Job.
- **Pull → queue → dispatch.** The `scheduler` periodically enqueues sync/reindex tasks (the
  "app pulls GitHub" path) with `source = 'schedule'` and no delivery id. They flow through the same
  `tasks` queue and the same dispatcher (see the schema note on `github_delivery_id` below).

Why this shape: returning `202` immediately and doing the heavy work out-of-band is the standard,
correct way to absorb GitHub's bursty at-least-once deliveries. Because *enqueue is just an
`INSERT`* in the same transaction as the rest of the handler's writes, there is no second system to
keep in sync — the dual-write problem never arises.

## Reference-level explanation

### The three problems are distinct — keep their mechanisms distinct

The current sketch tends to merge these; they need different tools and only one of them is on the
correctness path for "can we run two replicas":

| Problem | What it actually asks | Mechanism |
|---|---|---|
| Webhook delivery dedup (`X-GitHub-Delivery`) | "Did I already *receive* this delivery?" | Postgres `github_deliveries`, `INSERT … ON CONFLICT DO NOTHING` |
| Task idempotency | "Should this unit of work *exist*?" | Postgres **unique constraint** on the normalized task key |
| Work distribution | "Who runs it, and when?" | Postgres queue (`tasks` + `SKIP LOCKED`) |

The load-bearing rule: **correctness lives in database constraints, never in a Redis lock.** A
Redlock-style lock is best-effort — GC pauses, clock skew, and partitions can let two holders
coexist — so it is fine for *reducing wasted work* and for leader election, but it must always have
a DB-level guard behind it. This matters because any queue (including a broker, if we ever add one)
is at-least-once: consumers will see duplicates, and the unique constraint is what makes that
harmless.

### Dedup (unblocks N `serve` replicas)

Replace the in-memory set with the already-documented durable path
([components-and-data-models.md](../components-and-data-models.md#postgres-schema) already defines
`github_deliveries` with `delivery_id` as the primary key):

```sql
INSERT INTO github_deliveries (delivery_id, event_name, installation_id, repository_id, payload_json)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (delivery_id) DO NOTHING;
-- 0 rows affected ⇒ duplicate delivery ⇒ return 202 and stop.
```

This is the entirety of Phase 0 and is independently shippable: it makes the existing webhook
handler correct under concurrency and lets the `serve` role run N replicas. It implements intent
already written in the docs and in the code comment — it is not a new design.

### Queue (`tasks` table)

Proposed additive columns on `tasks` (mirrored in
`services/control-plane/schema/control-plane.cstack`, per
[ADR-0005](../adr/0005-cratestack-schema-first-control-plane.md)):

| Column | Purpose |
|---|---|
| `attempts int NOT NULL DEFAULT 0` | retry accounting |
| `run_after timestamptz NOT NULL DEFAULT now()` | delay + exponential backoff |
| `run_epoch int NOT NULL DEFAULT 0` | re-run discriminator (see idempotency index below) |
| `source text NOT NULL DEFAULT 'webhook'` | task origin: `webhook` or `schedule` |
| `job_name text` | name of the dispatched Kubernetes Job; lets the reaper check liveness |
| `lease_owner text` / `lease_expires_at timestamptz` | claim a task for dispatch; renewed by heartbeat, reaped on expiry (see Reaper) |

The `tasks` schema also needs one **modification**: `github_delivery_id` becomes **nullable**.
Today it is a `NOT NULL` foreign key to `github_deliveries`, which is right for webhook-originated
tasks but blocks the pull path — scheduler-produced sync/reindex tasks have no `X-GitHub-Delivery`
to reference. Making it nullable (with origin carried in `source`) lets both flows share one queue
without inventing synthetic delivery rows.

Plus a unique index enforcing "at most one task per normalized command + target + head SHA"
([idempotency model](../github-app-and-control-plane.md#idempotency-model)). Two wrinkles the index
must handle:

- **Nullable `head_sha`.** Postgres treats `NULL`s as distinct in a unique index by default, so a
  plain index would let duplicate tasks slip through whenever the head SHA is unknown. Use
  `NULLS NOT DISTINCT` (Postgres 15+) so the guard holds for null SHAs too.
- **Explicit re-runs.** Re-run commands are allowed to create a new version, so the key carries a
  `run_epoch` discriminator rather than being globally unique on the natural key.

```sql
CREATE UNIQUE INDEX tasks_idempotency_idx
  ON tasks (repository_id, target_type, target_id, command_text, head_sha, run_epoch)
  NULLS NOT DISTINCT;  -- Postgres 15+; pre-15: COALESCE(head_sha, '') in the key or a partial index
```

A matching partial index keeps the `SKIP LOCKED` dequeue from degrading to a table scan as the
table grows:

```sql
CREATE INDEX tasks_queue_idx ON tasks (priority DESC, created_at)
  WHERE status = 'queued';
```

Dispatcher dequeue — the whole concurrency story is one query; `SKIP LOCKED` guarantees that
multiple dispatcher replicas never claim the same row:

```sql
UPDATE tasks
SET status = 'running', attempts = attempts + 1,
    lease_owner = $1, lease_expires_at = now() + interval '1 minute', started_at = now()
WHERE id = (
  SELECT id FROM tasks
  WHERE status = 'queued' AND run_after <= now()
  ORDER BY priority DESC, created_at
  FOR UPDATE SKIP LOCKED
  LIMIT 1
)
RETURNING *;  -- short *claim* lease; only covers Job creation, then renewed by heartbeat (see below)
```

To avoid busy-polling, dispatchers `LISTEN` on a channel that `serve`/`scheduler` `NOTIFY` on
enqueue, with a short poll (1–5s) as a fallback so a missed notification never strands work. On
claim, the dispatcher creates the Job (ADR-0004), records `job_name` on the task, and transitions it
through the existing `Running → PostingResult → Succeeded/Failed/TimedOut` states as the Job reports
back.

Lease and reaping — **the Kubernetes Job is the source of truth for liveness, not a timer.** The
claim lease above is deliberately short: it only covers the window in which the dispatcher is
creating the Job. Once the Job exists, the dispatcher **renews the lease by heartbeat** for as long
as the Job is alive, so the lease never has to be pre-sized to a task's worst-case runtime (an
indexer Job may run up to `activeDeadlineSeconds` = 3600s). The **reaper** (scheduler) returns a task
to `queued` only when **both** conditions hold: `lease_expires_at` has passed **and** no live Job
named `job_name` exists. That liveness check is what prevents a still-running task from being
prematurely reclaimed and a second Job being spawned for it — preserving the one-Job-per-task
invariant of [ADR-0004](../adr/0004-one-k8s-job-per-task.md). The same check covers dead dispatchers
(claimed but never created a Job — lease expires, no Job → requeue), lost `NOTIFY`s, and orphaned
Jobs, and is what makes a lost message self-healing.

Retry/failure: on a recoverable failure set `status = 'queued'`, `run_after = now() + backoff`
until `attempts >= max`, then `failed`.

### Roles as one binary

```
control-plane serve        # api: webhook + REST (N replicas)
control-plane dispatcher   # claim queued tasks → create k8s Jobs
control-plane scheduler    # periodic pulls + reaper (singleton)
```

One image, three Deployments with different args, scaled independently. The `scheduler` stays a
single replica initially (simplest correct option); a k8s `CronJob` or a Redis leader-lease is the
upgrade path if it ever needs HA.

### Redis (added last; off the correctness path)

- cache for GitHub API responses and minted installation access tokens (TTL = token expiry)
- rate-limit counters to respect GitHub API limits across replicas
- optional scheduler leader-lease and a best-effort per-repo work lock (an optimization to avoid
  redundant clones, never a correctness guarantee)

Because `SKIP LOCKED` already provides the mutual exclusion for task pickup, Redis is **not**
required for any phase before Phase 3 and the system is correct without it.

### Deployment

Replica counts live in the external `ADORSYS-GIS/ai-helm` GitOps chart, not in this repo
(`deploy/envs/production/values.yaml` only pins image tags). The "control plane = 1 replica" setting
there is the operational expression of the code constraint above. Once Phase 0 lands, that value can
be raised; workers/dispatchers can later get an HPA on queue depth (`COUNT(*) WHERE status='queued'`).

### Phasing (each step independently shippable, each leaves the system correct)

- **Phase 0** — Replace the in-memory `seen_deliveries` set with Postgres `ON CONFLICT`. Unblocks
  N `serve` replicas. No queue, no Redis, no role split.
- **Phase 1** — Add the queue columns + idempotency index; add the `dispatcher` subcommand and the
  `SKIP LOCKED` dequeue; `serve` becomes enqueue-and-`202`.
- **Phase 2** — Add the `scheduler` subcommand: periodic pulls + reaper.
- **Phase 3** — Add Redis for GitHub-API / installation-token caching and rate limiting (pure
  performance; correctness does not depend on it).
- **Phase 4** — Raise replica counts in `ai-helm`; add an HPA on queue depth.

## Drawbacks

- A Postgres-backed queue has a throughput ceiling lower than a dedicated broker; very high event
  volumes or complex fan-out/routing could eventually justify revisiting (see Alternatives).
- Polling/`LISTEN`-`NOTIFY` adds a small dispatch latency versus a push broker.
- Three Deployments instead of one is marginally more operational surface (manifests, dashboards),
  though they share one image and one codebase.
- Redis, once added, is another stateful dependency to run — mitigated by keeping it strictly
  off the correctness path so an outage degrades performance, not correctness.

## Alternatives

- **RabbitMQ / Kafka broker.** Standalone, durable, feature-rich (routing, priorities, DLQ). Rejected
  for now: it is a third stateful system to operate **and** reintroduces the dual-write problem
  (write task to Postgres *and* publish to the broker, with no shared transaction) that the
  pg-queue avoids entirely. The broker earns its keep only past throughput/routing needs we do not
  have; revisit if event volume or fan-out grows materially.
- **Redis Streams as the queue.** Since Redis is being added anyway, Streams + consumer groups could
  carry the queue. Rejected: less durable than Postgres, and it would move correctness-bearing state
  off the database that is already our source of truth.
- **Shared long-lived worker pool instead of per-task Jobs.** Already considered and rejected by
  [ADR-0004](../adr/0004-one-k8s-job-per-task.md) on isolation grounds; this RFC deliberately keeps
  per-task Jobs and only adds the dispatcher in front of them.
- **Do nothing.** The control plane stays single-replica: no rolling-deploy redundancy, a single
  point of failure for webhook ingress, and no horizontal headroom. Not acceptable for production.

## Unresolved questions

- Whether `run_epoch` is the right re-run discriminator vs. a monotonic `version`, and how explicit
  re-run commands set it.
- Whether the `dispatcher` runs as 1 replica (simplest) or N from the start — `SKIP LOCKED` makes N
  safe, but Job-creation concurrency limits need a global cap (DB-counted vs. Redis-counted).
- Whether the `scheduler` begins as a single-replica Deployment, a k8s `CronJob`, or a leader-leased
  role — to be resolved in Phase 2.
- Claim-lease duration, heartbeat-renewal interval, backoff constants, and the reaper interval — to
  be tuned during implementation. (The Job-liveness check, not these timers, is what guarantees
  one Job per task.)
- Whether the reaper checks Job liveness via the Kubernetes API directly or via a Job-status field
  the dispatcher mirrors into Postgres.
- Whether Redis is needed at all before real GitHub rate-limit pressure is observed (Phase 3 may be
  deferred).
