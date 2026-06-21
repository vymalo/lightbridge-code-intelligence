-- Persisted review output (Epic #75, Milestone C).
--
-- A posted review currently lives only on GitHub. Persist a copy so the admin console can show what
-- the agent said for a run (and as an AI-governance audit record). One row per task (a task runs
-- once; a re-review is a new task), upserted on re-post.
CREATE TABLE IF NOT EXISTS reviews (
    task_id            uuid PRIMARY KEY REFERENCES tasks (id) ON DELETE CASCADE,
    summary            text        NOT NULL,
    -- The rendered review body (summary + deferred findings + disclosures) as posted to GitHub.
    body               text        NOT NULL,
    inline_count       int         NOT NULL DEFAULT 0,
    deferred_count     int         NOT NULL DEFAULT 0,
    out_of_scope_count int         NOT NULL DEFAULT 0,
    -- The agent's full findings array (file/line/severity/title/body), verbatim.
    findings           jsonb       NOT NULL DEFAULT '[]'::jsonb,
    created_at         timestamptz NOT NULL DEFAULT now()
);
