# ADR-0004: One Kubernetes Job per task for execution isolation

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Each review/Q&A task runs an agent that executes potentially untrusted reasoning over repository
content (including PRs from forks). We need an execution model that isolates tasks from one another,
cleans up reliably, and lets us scope credentials per task.

## Decision Drivers

- Isolation between tasks (especially untrusted forks)
- Predictable cleanup and resource bounding
- Per-task credentials (short-lived installation tokens)
- Debuggability of individual runs

## Considered Options

- **One Kubernetes Job per task** — strong isolation, TTL-based cleanup, per-task service accounts
  and secrets, per-task resource limits.
- **Shared long-lived worker pool** — lower startup latency, but weaker isolation and harder
  cleanup/debugging.

## Decision Outcome

Chosen option: **one Kubernetes Job per task.** Each task gets its own Job with
`ttlSecondsAfterFinished` for cleanup, `activeDeadlineSeconds` for bounding, a least-privilege
service account, and task-scoped credentials.

### Consequences

- Good, because tasks cannot interfere with each other and untrusted forks are contained.
- Good, because finished Jobs are cleaned up automatically and credentials are short-lived.
- Bad, because per-task pod startup adds latency compared to a warm shared pool.
- Neutral, because a shared pool remains a possible later optimization for trusted, high-volume
  workloads.
