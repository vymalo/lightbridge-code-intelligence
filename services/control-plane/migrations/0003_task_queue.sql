-- RFC-0001 Phase 1: make the `tasks` table a work queue consumed by the dispatcher via
-- SELECT ... FOR UPDATE SKIP LOCKED, and make webhook task creation idempotent.
-- (cratestack codegen deferred — ADR-0005; mirrored in schema/control-plane.cstack.)

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS attempts         INT         NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS run_after        TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS run_epoch        INT         NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS lease_owner      TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS job_name         TEXT;

-- At most one task per normalized command + target + head SHA (per run epoch). NULLS NOT DISTINCT
-- (Postgres 15+) so a null head_sha cannot bypass the guard; run_epoch lets an explicit re-run
-- create a new version later without colliding (RFC-0001).
CREATE UNIQUE INDEX IF NOT EXISTS tasks_idempotency_idx
    ON tasks (repository_id, target_type, target_id, command_text, head_sha, run_epoch)
    NULLS NOT DISTINCT;

-- Keep the dispatcher's SKIP LOCKED dequeue off a sequential scan as the table grows. Partial on
-- the only status the dispatcher selects.
CREATE INDEX IF NOT EXISTS tasks_queue_idx
    ON tasks (priority DESC, created_at)
    WHERE status = 'queued';
