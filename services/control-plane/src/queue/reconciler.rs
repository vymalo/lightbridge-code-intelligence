//! The `reconciler` role (ADR-0058) — bidirectional GitHub reconciliation in one single-replica loop:
//!
//! - **outbound (ADR-0059):** the **sole** GitHub egress. It drains `github_outbox` — the intent rows
//!   serve/finalize, the reaper, and the webhook 👀 enqueue — and posts each via the App key, marking it
//!   `posted` (recording the id for the feedback join) or backing it off on failure. NOTIFY-driven with a
//!   timer fallback, exactly like the dispatcher on `task_queued`.
//! - **inbound (ADR-0035):** reads 👍/👎 reactions on the comments we posted and reconciles them into
//!   `review_feedback` (GitHub emits no webhook for reactions).
//!
//! Single replica is load-bearing: it makes "sole consumer" literal and keeps the outbox's per-task
//! ordering intact. The role is the only one besides serve that holds the App key (ADR-0002).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgListener;
use sqlx::PgPool;

use crate::config::ReviewSection;
use crate::integrations::github::{GithubApp, ReviewComment};

/// How many intents to claim per drain pass.
const DRAIN_BATCH: i64 = 50;
/// Fallback wake if a `NOTIFY github_outbox` is missed (e.g. fired while we were mid-batch).
const DRAIN_FALLBACK: Duration = Duration::from_secs(15);

/// Run the reconciler: the outbox drain (foreground) plus the feedback poll (spawned). Either failing a
/// cycle is logged and retried — a transient GitHub/DB blip must not kill the role.
pub async fn run(
    pool: PgPool,
    app: GithubApp,
    review: Arc<ReviewSection>,
    interval: Duration,
    within_days: i32,
) -> anyhow::Result<()> {
    // Feedback poll (ADR-0035) on its own cadence, alongside the drain.
    {
        let (pool, app) = (pool.clone(), app.clone());
        let interval_secs = interval.as_secs() as i64;
        tokio::spawn(async move {
            tracing::info!(
                interval_secs,
                within_days,
                "reconciler: feedback poll started"
            );
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                match poll_once(&pool, &app, within_days, interval_secs).await {
                    Ok(n) => tracing::debug!(comments = n, "feedback poll cycle complete"),
                    Err(error) => tracing::warn!(%error, "feedback poll cycle failed (will retry)"),
                }
            }
        });
    }
    run_outbox_drain(pool, app, review).await
}

/// The GitHub-egress drain loop (ADR-0059): wake on `NOTIFY github_outbox` (timer fallback), then drain
/// every due intent before sleeping again.
async fn run_outbox_drain(
    pool: PgPool,
    app: GithubApp,
    review: Arc<ReviewSection>,
) -> anyhow::Result<()> {
    let mut listener = PgListener::connect_with(&pool).await?;
    listener.listen(crate::db::GITHUB_OUTBOX_CHANNEL).await?;
    tracing::info!("reconciler: github-egress drain started");
    loop {
        // Drain everything currently due, in batches, before parking on the listener.
        loop {
            match drain_once(&pool, &app, &review).await {
                Ok(0) => break,
                Ok(n) => tracing::debug!(posted = n, "outbox drain batch"),
                Err(error) => {
                    tracing::warn!(%error, "outbox drain failed (will retry on next wake)");
                    break;
                }
            }
        }
        tokio::select! {
            res = listener.recv() => {
                if let Err(error) = res {
                    tracing::warn!(%error, "outbox LISTEN dropped; relying on timer fallback");
                    tokio::time::sleep(DRAIN_FALLBACK).await;
                }
            }
            _ = tokio::time::sleep(DRAIN_FALLBACK) => {}
        }
    }
}

/// Claim one batch and deliver each intent. Marks every row `posted` (with the returned id) or backs it
/// off `failed`, so the row never re-claims unbounded — including a token-mint failure. Returns how many
/// posted.
async fn drain_once(
    pool: &PgPool,
    app: &GithubApp,
    review: &ReviewSection,
) -> anyhow::Result<usize> {
    let rows = crate::db::claim_outbox_batch(pool, DRAIN_BATCH).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    // One token per installation per batch; `None` caches a mint failure.
    let mut tokens: HashMap<i64, Option<String>> = HashMap::new();
    let mut posted = 0;
    for row in rows {
        let token = match tokens.get(&row.installation_id) {
            Some(Some(t)) => t.clone(),
            Some(None) => {
                // Mint already failed this batch — back the row off rather than spin on it.
                let _ = crate::db::mark_outbox_failed(pool, row.id, "mint token failed").await;
                continue;
            }
            None => match app.installation_token(row.installation_id).await {
                Ok(t) => {
                    tokens.insert(row.installation_id, Some(t.clone()));
                    t
                }
                Err(error) => {
                    tracing::warn!(%error, installation = row.installation_id, "outbox: mint token failed");
                    tokens.insert(row.installation_id, None);
                    let _ = crate::db::mark_outbox_failed(pool, row.id, "mint token failed").await;
                    continue;
                }
            },
        };
        if row.attempts > 0 {
            tracing::info!(outbox_id = row.id, attempts = row.attempts, kind = %row.kind, "outbox: retrying delivery");
        }
        match deliver(pool, app, &token, review, &row).await {
            Ok(github_id) => {
                if let Err(error) = crate::db::mark_outbox_posted(pool, row.id, github_id).await {
                    tracing::warn!(%error, outbox_id = row.id, "marking outbox posted failed");
                }
                posted += 1;
            }
            Err(error) => {
                tracing::warn!(%error, outbox_id = row.id, kind = %row.kind, "outbox delivery failed (will back off)");
                let _ = crate::db::mark_outbox_failed(pool, row.id, &error.to_string()).await;
            }
        }
    }
    Ok(posted)
}

/// Post one intent. Returns the GitHub id to record (review/comment) or `None`. An `Err` backs the row
/// off for retry.
async fn deliver(
    pool: &PgPool,
    app: &GithubApp,
    token: &str,
    review: &ReviewSection,
    row: &crate::db::OutboxRow,
) -> anyhow::Result<Option<i64>> {
    match row.kind.as_str() {
        "reaction" => {
            let issue = payload_i64(&row.payload, "issue")?;
            let content = payload_str(&row.payload, "content")?;
            app.add_reaction(token, &row.owner, &row.repo, issue, content)
                .await?;
            Ok(None)
        }
        "reply" => {
            let issue = payload_i64(&row.payload, "issue")?;
            let body = payload_str(&row.payload, "body")?;
            let posted = app
                .create_issue_comment(token, &row.owner, &row.repo, issue, body)
                .await?;
            record_comment(pool, row.task_id, posted.id, "reply").await;
            Ok(posted.id)
        }
        "failure_notice" => {
            // Re-check the ADR-0056 dedup gate at post time: a finalize-then-fail may have already posted
            // a real review, in which case there's nothing to apologise for — consume the row silently.
            if let Some(task) = row.task_id {
                if crate::db::has_posted_to_github(pool, task)
                    .await
                    .unwrap_or(false)
                {
                    return Ok(None);
                }
            }
            let issue = payload_i64(&row.payload, "issue")?;
            let body = payload_str(&row.payload, "body")?;
            let posted = app
                .create_issue_comment(token, &row.owner, &row.repo, issue, body)
                .await?;
            record_comment(pool, row.task_id, posted.id, "failure_notice").await;
            Ok(posted.id)
        }
        "review" => deliver_review(pool, app, token, review, row).await,
        other => anyhow::bail!("unknown outbox kind {other:?}"),
    }
}

/// Post the grouped review and its success side-effects (persist the copy, fetch inline ids, apply
/// outcome labels + 🎉) — the whole bundle the old synchronous `finalize_review` did, now driven from
/// the pre-shaped payload.
async fn deliver_review(
    pool: &PgPool,
    app: &GithubApp,
    token: &str,
    review: &ReviewSection,
    row: &crate::db::OutboxRow,
) -> anyhow::Result<Option<i64>> {
    let p: crate::outbox::ReviewPayload = serde_json::from_value(row.payload.clone())?;
    let comments: Vec<ReviewComment> = p
        .comments
        .iter()
        .map(|c| ReviewComment {
            path: c.path.clone(),
            line: c.line,
            side: "RIGHT",
            body: c.body.clone(),
        })
        .collect();
    let posted = app
        .create_pr_review(token, &row.owner, &row.repo, p.pr, &p.body, &comments)
        .await?;
    tracing::info!(
        outbox_id = row.id,
        pr = p.pr,
        inline = p.inline_n,
        "review posted"
    );

    if let Some(task) = row.task_id {
        if let Err(error) = crate::db::upsert_review(
            pool,
            task,
            &p.summary,
            &p.body,
            p.inline_n,
            p.deferred_n,
            p.out_of_scope_n,
            &p.findings_json,
            posted.html_url.as_deref(),
            posted.id,
        )
        .await
        {
            tracing::warn!(%error, task_id = %task, "persisting review copy failed (non-fatal)");
        }
        // Inline comment ids (the create-review response omits them) for the feedback join.
        if let Some(review_id) = posted.id {
            match app
                .list_review_comments(token, &row.owner, &row.repo, p.pr, review_id)
                .await
            {
                Ok(refs) => {
                    let stored: Vec<crate::db::ReviewCommentRef> = refs
                        .into_iter()
                        .map(|c| crate::db::ReviewCommentRef {
                            github_comment_id: c.id,
                            kind: "inline".to_string(),
                            file: c.path,
                            line: c.line.map(|l| l as i32),
                        })
                        .collect();
                    if let Err(error) = crate::db::store_review_comments(pool, task, &stored).await
                    {
                        tracing::warn!(%error, task_id = %task, "storing review comment ids failed (non-fatal)");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, task_id = %task, "fetching review comment ids failed (non-fatal)")
                }
            }
        }
    }

    // Outcome labels (ADR rides the outbox, not a 2nd serve writer) + the 🎉 reaction — both best-effort.
    let mut labels = Vec::new();
    if let Some(l) = &review.label_reviewed {
        labels.push(l.clone());
    }
    if p.label_findings {
        if let Some(l) = &review.label_findings {
            labels.push(l.clone());
        }
    }
    if p.label_error {
        if let Some(l) = &review.label_error {
            labels.push(l.clone());
        }
    }
    if !labels.is_empty() {
        if let Err(error) = app
            .add_labels(token, &row.owner, &row.repo, p.pr, &labels)
            .await
        {
            tracing::warn!(%error, pr = p.pr, "applying outcome labels failed (non-fatal)");
        }
    }
    if review.reactions_enabled() {
        if let Err(error) = app
            .add_reaction(token, &row.owner, &row.repo, p.pr, "hooray")
            .await
        {
            tracing::warn!(%error, pr = p.pr, "review 🎉 reaction failed (non-fatal)");
        }
    }
    Ok(posted.id)
}

/// Record a posted comment's id so the feedback poll can read its reactions (ADR-0035). Best-effort;
/// a missing id or store error just means that comment's reactions go unread.
async fn record_comment(pool: &PgPool, task_id: Option<uuid::Uuid>, id: Option<i64>, kind: &str) {
    let (Some(task), Some(cid)) = (task_id, id) else {
        return;
    };
    if let Err(error) = crate::db::store_review_comments(
        pool,
        task,
        &[crate::db::ReviewCommentRef {
            github_comment_id: cid,
            kind: kind.to_string(),
            file: None,
            line: None,
        }],
    )
    .await
    {
        tracing::warn!(%error, task_id = %task, kind, "storing posted comment id failed (non-fatal)");
    }
}

fn payload_i64(v: &serde_json::Value, key: &str) -> anyhow::Result<i64> {
    v.get(key)
        .and_then(|x| x.as_i64())
        .ok_or_else(|| anyhow::anyhow!("outbox payload missing i64 {key:?}"))
}

fn payload_str<'a>(v: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("outbox payload missing str {key:?}"))
}

/// One feedback poll cycle: for each comment due this cycle (age-tiered), read its reactions and
/// reconcile. Returns the number checked. Mints at most one token per installation per cycle.
async fn poll_once(
    pool: &PgPool,
    app: &GithubApp,
    within_days: i32,
    interval_secs: i64,
) -> anyhow::Result<usize> {
    let comments = crate::db::list_pollable_comments(pool, within_days, interval_secs).await?;
    let mut tokens: HashMap<i64, Option<String>> = HashMap::new();
    let mut checked = 0;
    for c in &comments {
        let token = match tokens.get(&c.installation_id) {
            Some(Some(t)) => t.clone(),
            Some(None) => continue,
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
