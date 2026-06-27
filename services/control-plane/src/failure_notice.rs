//! Posting the ADR-0056 "review failed, try again" fallback notice — shared by the two paths that
//! detect a terminally-failed PR review task and owe its author a word instead of silence:
//!
//! 1. **serve** ([`crate::http`]'s `handle_review_failure`) — the runner *reports* `failed`/`timed_out`
//!    and serve posts synchronously on that status report.
//! 2. **poller** ([`crate::queue::poller`]) — a sweep that catches an *uncatchable* kill (OOM / SIGKILL
//!    / node eviction) the runner could never report. The reaper marks such a task `failed` in the
//!    **dispatcher**, which holds no GitHub App key (ADR-0002) and so cannot post; the key-holding
//!    poller posts the notice instead (ADR-0057).
//!
//! Both call [`post_if_unposted`], which is idempotent: it posts only when nothing has been posted for
//! the task yet (no review, reply, or prior notice — [`crate::db::has_posted_to_github`]) and records
//! the notice as a `failure_notice` comment so the other path, or a later retry, never double-posts.

use sqlx::PgPool;
use uuid::Uuid;

use crate::db::TaskContextRow;
use crate::integrations::github::GithubApp;

/// Post the failure notice for PR task `id` **iff** nothing has been posted for it yet, returning
/// `true` when a notice actually went out this call. The caller has already loaded `context` (and
/// guarantees `context.target_type == "pull_request"`) and minted `token` for
/// `context.installation_id` — both callers need the token for their own surrounding work, so passing
/// it in avoids a redundant mint.
///
/// Best-effort: every error is logged and swallowed — the notice is a courtesy, never load-bearing,
/// and must not turn a failed review into a failed *role*.
pub async fn post_if_unposted(
    pool: &PgPool,
    app: &GithubApp,
    token: &str,
    context: &TaskContextRow,
    id: Uuid,
) -> bool {
    // Dedup gate: a real review/answer, or a prior notice, means there's nothing to apologise for.
    match crate::db::has_posted_to_github(pool, id).await {
        Ok(true) => return false,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, task_id = %id, "failure notice: posted-check failed");
            return false;
        }
    }
    let body = crate::review::render_failure_notice();
    let posted = match app
        .create_issue_comment(
            token,
            &context.owner,
            &context.name,
            context.target_id,
            &body,
        )
        .await
    {
        Ok(posted) => posted,
        Err(error) => {
            tracing::warn!(%error, task_id = %id, "posting failure notice failed (non-fatal)");
            return false;
        }
    };
    tracing::info!(task_id = %id, "posted failure notice — review did not finalize (ADR-0056)");
    // Record it so the other path / a retry doesn't post a second notice (dedup via
    // has_posted_to_github). If GitHub returned no id, or the store fails, the dedup row is missing —
    // log it loudly, since a later attempt could then post a duplicate notice (#215 review).
    match posted.id {
        Some(cid) => {
            if let Err(error) = crate::db::store_review_comments(
                pool,
                id,
                &[crate::db::ReviewCommentRef {
                    github_comment_id: cid,
                    kind: "failure_notice".to_string(),
                    file: None,
                    line: None,
                }],
            )
            .await
            {
                tracing::warn!(%error, task_id = %id, "recording failure notice for dedup failed — a retry may post a duplicate");
            }
        }
        None => tracing::warn!(
            task_id = %id,
            "failure notice posted but GitHub returned no comment id — dedup not recorded; a retry may duplicate"
        ),
    }
    true
}
