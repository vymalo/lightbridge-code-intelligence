-- Repo approval gate (Epic #75, Milestone A).
--
-- The GitHub App may be installed on any org/repo, but we must NOT index or review a repository until
-- an admin has explicitly approved it — otherwise anyone could point the tool at private repos. This
-- adds an approval `status` to `repositories`; task creation is gated on `status = 'approved'`.
--
-- `status`: 'pending' (newly seen via an installation/PR webhook — awaiting admin approval),
-- 'approved' (admin opted it in — work may run), 'disabled' (removed from the installation or denied
-- — no work; index data is purged separately in Milestone B).
ALTER TABLE repositories
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'disabled'));
ALTER TABLE repositories ADD COLUMN IF NOT EXISTS approved_at timestamptz;
ALTER TABLE repositories ADD COLUMN IF NOT EXISTS approved_by TEXT;

-- Grandfather every repository that already existed before the gate to 'approved', so the current
-- production behaviour is unchanged — only repos added AFTER this migration require approval.
UPDATE repositories SET status = 'approved' WHERE status = 'pending';

-- The pending queue is small but read on every admin poll; index it for the status filter.
CREATE INDEX IF NOT EXISTS repositories_status_idx ON repositories (status);
