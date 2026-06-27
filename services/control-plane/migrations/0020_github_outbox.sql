-- ADR-0059: the single GitHub egress. Every outbound GitHub *content* write becomes an intent row here;
-- the reconciler (ADR-0058) is the sole consumer that posts them. Producers (serve/finalize, the reaper,
-- the webhook 👀) only INSERT — they never call the GitHub write API.
--
-- owner/repo/installation_id are carried as columns (not joined from tasks) so the reconciler needs no
-- join to post, and a reaction can be enqueued even when it isn't tied to a task row. task_id is nullable
-- for that reason, but is set for review/reply/failure_notice so the posted ids can be recorded back
-- (ADR-0035 feedback join).
CREATE TABLE IF NOT EXISTS github_outbox (
    id              BIGSERIAL PRIMARY KEY,
    task_id         UUID REFERENCES tasks (id) ON DELETE CASCADE,
    installation_id BIGINT NOT NULL,
    owner           TEXT   NOT NULL,
    repo            TEXT   NOT NULL,
    kind            TEXT   NOT NULL CHECK (kind IN ('review', 'reply', 'reaction', 'label', 'failure_notice')),
    payload         JSONB  NOT NULL,
    -- One intent per logical post: a stable key makes enqueue idempotent (ON CONFLICT DO NOTHING), so a
    -- re-finalize or a retry never double-enqueues. e.g. '<task>:review', '<task>:reaction:eyes'.
    dedup_key       TEXT   NOT NULL UNIQUE,
    status          TEXT   NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'posted', 'failed')),
    attempts        INT    NOT NULL DEFAULT 0,
    last_error      TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    posted_at       TIMESTAMPTZ,
    github_id       BIGINT
);

-- The drain claim: pending rows that are due, in (created_at, id) order — id breaks created_at ties since
-- rows enqueued in one transaction share now() (transaction-stable). Partial on status so it stays small.
CREATE INDEX IF NOT EXISTS github_outbox_drain_idx
    ON github_outbox (next_attempt_at, created_at, id)
    WHERE status = 'pending';
