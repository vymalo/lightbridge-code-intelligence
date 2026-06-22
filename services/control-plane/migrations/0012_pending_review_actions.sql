-- ADR-0037: the agent acts via mediated write tools (add_review_comment / add_comment / set_summary)
-- it calls *during* the run; the control plane accumulates them here and flushes once on clean
-- completion (one grouped PR review + a single consolidated reply), so a mid-run crash posts nothing.
--
-- Dedup + idempotency (ADR-0037, refined per the #155 review):
--   - inline findings are deduped by (task_id, file, line) — last write wins — NOT a content hash,
--     since a non-deterministic LLM re-run rewords the same finding;
--   - the summary is single-valued per task (last write wins);
--   - plain comments are append-only, ordered by `id`, consolidated into one reply at flush.
-- The whole buffer for a task is cleared when its runner (re)starts, so a retry begins from empty.
CREATE TABLE IF NOT EXISTS pending_review_actions (
    id         BIGSERIAL   PRIMARY KEY,
    task_id    UUID        NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    -- 'inline' (a diff-pinned finding), 'comment' (a plain thread reply), or 'summary' (the verdict).
    action     TEXT        NOT NULL CHECK (action IN ('inline', 'comment', 'summary')),
    file       TEXT,                 -- inline only
    line       INT,                  -- inline only
    title      TEXT,                 -- inline only (short finding title)
    priority   TEXT,                 -- inline only (P0|P1|P2)
    category   TEXT,                 -- inline only
    suggestion TEXT,                 -- inline only (exact replacement for `line`)
    body       TEXT        NOT NULL, -- inline body / comment body / summary text
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- An inline finding must carry a location; enforce it at the DB level (defense in depth — the
    -- endpoint already requires file+line).
    CONSTRAINT inline_has_location CHECK (action <> 'inline' OR (file IS NOT NULL AND line IS NOT NULL))
);

-- One inline finding per (task, file, line): a re-emitted finding overwrites rather than duplicates.
CREATE UNIQUE INDEX IF NOT EXISTS pending_review_inline_uniq
    ON pending_review_actions (task_id, file, line)
    WHERE action = 'inline';

-- At most one summary per task (last write wins).
CREATE UNIQUE INDEX IF NOT EXISTS pending_review_summary_uniq
    ON pending_review_actions (task_id)
    WHERE action = 'summary';

-- Read/clear the whole buffer for a task efficiently.
CREATE INDEX IF NOT EXISTS pending_review_actions_task_idx
    ON pending_review_actions (task_id);
