-- Index the tasks → github_deliveries foreign key. Postgres does not auto-index FK columns, so
-- lookups/joins by delivery id (and FK integrity checks) would otherwise scan. Added as a separate
-- migration because 0001 is already applied (SQLx checksums applied migrations — never edit them).
CREATE INDEX IF NOT EXISTS tasks_github_delivery_id_idx ON tasks (github_delivery_id);
