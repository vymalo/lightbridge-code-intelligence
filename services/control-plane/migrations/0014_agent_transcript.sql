-- ADR-0034: persist the agent run transcript (tool calls, reasoning, token usage) so a run is
-- inspectable — why the review said what it did. The runner submits the whole transcript once at the
-- end of a run (success or failure); the control plane stores it ordered by `seq`. Re-submitting (a
-- task retry) replaces the prior rows for the task. The dashboard timeline (apps/web) consumes it
-- later; this migration + the ingest/read API are the backend half.
--
-- NOTE: numbered 0014 to sit after #144's 0013_review_github_id.sql — merge that PR (#161) first so
-- the migration sequence has no gap in prod.
CREATE TABLE IF NOT EXISTS agent_transcript (
    id                UUID        PRIMARY KEY,
    task_id           UUID        NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    seq               INT         NOT NULL,          -- order within the run (0-based)
    role              TEXT        NOT NULL,          -- 'assistant' | 'tool'
    content           TEXT,                          -- reasoning text / tool result (truncated)
    tool_calls        JSONB,                         -- the assistant turn's tool_calls array
    tool_name         TEXT,                          -- for a tool-result row: which tool
    prompt_tokens     BIGINT,                         -- assistant turn token usage (when reported)
    completion_tokens BIGINT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS agent_transcript_task_seq_idx ON agent_transcript (task_id, seq);
