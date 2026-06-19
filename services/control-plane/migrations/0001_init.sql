-- Initial control-plane schema. Hand-written SQLx (cratestack codegen deferred — ADR-0005);
-- mirrors schema/control-plane.cstack. Applied automatically on startup via `sqlx::migrate!`.

CREATE TABLE IF NOT EXISTS repositories (
    id             BIGSERIAL PRIMARY KEY,
    github_repo_id BIGINT  NOT NULL UNIQUE,
    owner          TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    default_branch TEXT    NOT NULL,
    active         BOOLEAN NOT NULL DEFAULT TRUE
);

-- X-GitHub-Delivery is the natural idempotency key: the PRIMARY KEY + ON CONFLICT gives us
-- exactly-once delivery handling without an in-process dedup set.
CREATE TABLE IF NOT EXISTS github_deliveries (
    delivery_id     TEXT PRIMARY KEY,
    event_name      TEXT        NOT NULL,
    installation_id BIGINT,
    repository_id   BIGINT,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    payload_json    JSONB       NOT NULL
);

CREATE TABLE IF NOT EXISTS repo_index (
    id             BIGSERIAL PRIMARY KEY,
    repository_id  BIGINT NOT NULL REFERENCES repositories (id),
    branch         TEXT   NOT NULL,
    commit_sha     TEXT   NOT NULL,
    graph_version  TEXT   NOT NULL,
    vector_version TEXT   NOT NULL,
    status         TEXT   NOT NULL,
    started_at     TIMESTAMPTZ,
    completed_at   TIMESTAMPTZ,
    UNIQUE (repository_id, branch, commit_sha, graph_version, vector_version)
);

CREATE TABLE IF NOT EXISTS tasks (
    id                 UUID PRIMARY KEY,
    repository_id      BIGINT NOT NULL REFERENCES repositories (id),
    installation_id    BIGINT NOT NULL,
    github_delivery_id TEXT   NOT NULL REFERENCES github_deliveries (delivery_id),
    target_type        TEXT   NOT NULL,
    target_id          BIGINT NOT NULL,
    command_text       TEXT   NOT NULL,
    base_sha           TEXT,
    head_sha           TEXT,
    status             TEXT   NOT NULL,
    priority           INT    NOT NULL DEFAULT 100,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- run timing for the dashboard's task-run views (ADR-0016)
    started_at         TIMESTAMPTZ,
    completed_at       TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS tasks_repo_status_idx ON tasks (repository_id, status);
