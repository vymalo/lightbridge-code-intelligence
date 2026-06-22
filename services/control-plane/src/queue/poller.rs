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
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        match poll_once(&pool, &app, within_days).await {
            Ok(n) => tracing::debug!(comments = n, "feedback poll cycle complete"),
            Err(error) => tracing::warn!(%error, "feedback poll cycle failed (will retry)"),
        }
    }
}

/// One poll cycle: for each recent comment we own, read its reactions and reconcile. Returns the
/// number of comments checked. Mints at most one installation token per installation per cycle.
async fn poll_once(pool: &PgPool, app: &GithubApp, within_days: i32) -> anyhow::Result<usize> {
    let comments = crate::db::list_pollable_comments(pool, within_days).await?;
    let mut tokens: HashMap<i64, String> = HashMap::new();
    let mut checked = 0;
    for c in &comments {
        // Mint (and cache for this cycle) an installation token; skip the comment if it fails.
        let token = match tokens.get(&c.installation_id) {
            Some(t) => t.clone(),
            None => match app.installation_token(c.installation_id).await {
                Ok(t) => {
                    tokens.insert(c.installation_id, t.clone());
                    t
                }
                Err(error) => {
                    tracing::warn!(%error, installation = c.installation_id, "mint token for poll failed");
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
