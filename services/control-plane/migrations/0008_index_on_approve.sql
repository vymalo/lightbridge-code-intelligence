-- Index-on-approve (Epic #75, Milestone B).
--
-- When an admin approves a repository we enqueue a standalone `index` task (target_type
-- `repository`, command `index`) that indexes the default branch — separate from per-PR reviews.
-- Two schema needs:
--
-- 1. The index task must mint an installation token to clone, so we need the repo's
--    `installation_id`. It's stable per installation; populate it from the installation /
--    installation_repositories / pull_request webhooks (which all carry installation.id). Nullable:
--    a repo seen only via an older path may not have it yet (then index-on-approve is skipped).
ALTER TABLE repositories ADD COLUMN IF NOT EXISTS installation_id BIGINT;

-- 2. An admin-initiated index task has no originating GitHub delivery, so `tasks.github_delivery_id`
--    can no longer be NOT NULL. Webhook-created tasks still set it.
ALTER TABLE tasks ALTER COLUMN github_delivery_id DROP NOT NULL;
