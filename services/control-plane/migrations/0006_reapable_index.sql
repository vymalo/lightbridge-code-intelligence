-- RFC-0001 Phase 2 (reaper): keep `list_reapable_tasks` off a sequential scan as the table grows.
-- The reaper runs every ~30s scanning for active tasks whose lease has expired; a partial index on
-- the active statuses keeps it tiny and the query fast (ORDER BY started_at).
CREATE INDEX IF NOT EXISTS tasks_reapable_idx
    ON tasks (lease_expires_at, started_at)
    WHERE status IN ('running', 'posting_result');
