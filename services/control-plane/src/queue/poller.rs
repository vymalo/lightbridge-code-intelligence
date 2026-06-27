//! Review-feedback poller (ADR-0035) — the `poller` role.
//!
//! GitHub does **not** emit webhooks for reactions, so a single-replica background loop periodically
//! reads the reactions on the comments we posted (`review_comments`) and reconciles them into
//! `review_feedback`. Reconciliation (insert new, delete vanished) is how we capture an un-react
//! without a webhook. The role is its own deployment so it — and only it — holds the GitHub App key
//! (single replica avoids double-polling; ADR-0002 keeps the credential off the multi-replica `serve`
//! pods and the dispatcher).

use std::collections::HashMap;
use std::time::Duration;

use sqlx::PgPool;

use crate::integrations::github::GithubApp;

/// Run the feedback poll loop forever, one cycle per `interval`. A cycle failure is logged and
/// retried next tick — a transient GitHub/DB blip must not kill the role.
pub async fn run(
    pool: PgPool,
    app: GithubApp,
    interval: Duration,
    within_days: i32,
) -> anyhow::Result<()> {
    tracing::info!(
        interval_secs = interval.as_secs(),
        within_days,
        "feedback poller started"
    );
    let interval_secs = interval.as_secs() as i64;
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        match poll_once(&pool, &app, within_days, interval_secs).await {
            Ok(n) => tracing::debug!(comments = n, "feedback poll cycle complete"),
            Err(error) => tracing::warn!(%error, "feedback poll cycle failed (will retry)"),
        }
        // Independent of the feedback poll: post the ADR-0057 fallback notice for any PR review that
        // died uncatchably and so never reported (a failure the keyless dispatcher couldn't apologise
        // for). A failure here is logged and retried next tick — it must not kill the role.
        match sweep_failure_notices(&pool, &app, within_days).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                posted = n,
                "failure-notice sweep posted notices for unreported kills"
            ),
            Err(error) => tracing::warn!(%error, "failure-notice sweep failed (will retry)"),
        }
    }
}

/// Backstop for ADR-0056 (ADR-0057): an *uncatchable* kill — OOM / SIGKILL / node eviction — never lets
/// the runner report its failure, so the reaper marks the task `failed` in the **dispatcher**, which
/// holds no GitHub App key (ADR-0002) and can't post. The poller can. Each cycle, find PR tasks that
/// ended terminally with nothing posted and post the "something went wrong, try again" notice.
///
/// Idempotent and race-free with the serve path: [`crate::db::failed_pr_tasks_without_feedback`] hides
/// a just-failed task behind a short settle buffer (serve posts synchronously on a *reported* failure),
/// and [`crate::failure_notice::post_if_unposted`] re-checks the same dedup gate before posting, so the
/// two paths never double-post. The notice's recorded row drops the task from the set next cycle.
/// Returns the number of notices posted this cycle.
async fn sweep_failure_notices(
    pool: &PgPool,
    app: &GithubApp,
    within_days: i32,
) -> anyhow::Result<usize> {
    let ids = crate::db::failed_pr_tasks_without_feedback(pool, within_days).await?;
    // Same per-installation token cache as the feedback poll: `None` caches a failed mint so one bad
    // installation isn't retried for every task in the batch.
    let mut tokens: HashMap<i64, Option<String>> = HashMap::new();
    let mut posted = 0;
    for id in ids {
        let context = match crate::db::get_task_context(pool, id).await {
            Ok(Some(c)) if c.target_type == "pull_request" => c,
            Ok(_) => continue, // task gone, or not a PR — nothing to apologise on
            Err(error) => {
                tracing::warn!(%error, task_id = %id, "failure-notice sweep: context load failed");
                continue;
            }
        };
        let token = match tokens.get(&context.installation_id) {
            Some(Some(t)) => t.clone(),
            Some(None) => continue, // mint already failed this cycle
            None => match app.installation_token(context.installation_id).await {
                Ok(t) => {
                    tokens.insert(context.installation_id, Some(t.clone()));
                    t
                }
                Err(error) => {
                    tracing::warn!(%error, installation = context.installation_id, "failure-notice sweep: mint token failed");
                    tokens.insert(context.installation_id, None);
                    continue;
                }
            },
        };
        if crate::failure_notice::post_if_unposted(pool, app, &token, &context, id).await {
            posted += 1;
        }
    }
    Ok(posted)
}

/// One poll cycle: for each comment due this cycle (age-tiered), read its reactions and reconcile.
/// Returns the number of comments checked. Mints at most one installation token per installation per
/// cycle — and caches a mint *failure* too, so one bad installation isn't retried for every comment.
async fn poll_once(
    pool: &PgPool,
    app: &GithubApp,
    within_days: i32,
    interval_secs: i64,
) -> anyhow::Result<usize> {
    let comments = crate::db::list_pollable_comments(pool, within_days, interval_secs).await?;
    // `None` caches a failed mint so subsequent comments for that installation skip immediately.
    let mut tokens: HashMap<i64, Option<String>> = HashMap::new();
    let mut checked = 0;
    for c in &comments {
        let token = match tokens.get(&c.installation_id) {
            Some(Some(t)) => t.clone(),
            Some(None) => continue, // mint already failed this cycle
            None => match app.installation_token(c.installation_id).await {
                Ok(t) => {
                    tokens.insert(c.installation_id, Some(t.clone()));
                    t
                }
                Err(error) => {
                    tracing::warn!(%error, installation = c.installation_id, "mint token for poll failed");
                    tokens.insert(c.installation_id, None);
                    continue;
                }
            },
        };
        let is_review_comment = c.kind == "inline";
        match app
            .list_comment_reactions(
                &token,
                &c.owner,
                &c.name,
                c.github_comment_id,
                is_review_comment,
            )
            .await
        {
            Ok(reactions) => {
                if let Err(error) = crate::db::reconcile_comment_feedback(
                    pool,
                    c.task_id,
                    c.github_comment_id,
                    &c.kind,
                    &reactions,
                )
                .await
                {
                    tracing::warn!(%error, comment = c.github_comment_id, "reconciling feedback failed");
                } else {
                    checked += 1;
                }
            }
            Err(error) => {
                tracing::warn!(%error, comment = c.github_comment_id, "reading reactions failed")
            }
        }
    }
    Ok(checked)
}
