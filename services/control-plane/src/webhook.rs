//! GitHub webhook receiver.
//!
//! Mirrors docs/github-app-and-control-plane.md: verify `X-Hub-Signature-256`, dedupe on
//! `X-GitHub-Delivery`, then hand off to task routing. With a database, dedup + persistence happen
//! atomically via the `github_deliveries` PRIMARY KEY; without one (dev) it falls back to an
//! in-memory set.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::AppState;

type HmacSha256 = Hmac<Sha256>;

pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let signature = header(&headers, "x-hub-signature-256");
    if !verify_signature(state.github_webhook_secret.as_bytes(), &body, &signature) {
        crate::metrics::webhook_signature_failure();
        tracing::warn!("invalid webhook signature");
        return (StatusCode::UNAUTHORIZED, "invalid signature");
    }

    let delivery_id = header(&headers, "x-github-delivery");
    if delivery_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing delivery id");
    }
    let event = header(&headers, "x-github-event");

    // Parse the payload up front: reject non-JSON bodies (never persist `null`), and have the
    // parsed value ready for the upcoming task-routing logic.
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(%error, delivery_id, "webhook payload is not valid JSON");
            return (StatusCode::BAD_REQUEST, "invalid json payload");
        }
    };

    // Dedup (and persist, when a database is configured). `is_new` is false for a replayed
    // delivery id — GitHub retries, so this is the exactly-once guard.
    let is_new = match &state.db {
        Some(pool) => {
            match crate::db::record_delivery(pool, &delivery_id, &event, &payload).await {
                Ok(is_new) => is_new,
                Err(error) => {
                    tracing::error!(%error, delivery_id, "failed to persist delivery");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "persistence error");
                }
            }
        }
        None => state
            .seen_deliveries
            .lock()
            .expect("dedup lock poisoned")
            .insert(delivery_id.clone()),
    };
    if !is_new {
        crate::metrics::webhook_duplicate();
        tracing::info!(delivery_id, "duplicate delivery");
        return (StatusCode::ACCEPTED, "duplicate delivery");
    }

    crate::metrics::webhook_delivery(&event);
    tracing::info!(delivery_id, event, "accepted webhook");

    // Webhook → internal action mapping (the only events that do anything beyond being persisted):
    //
    //   pull_request               opened                  → review task (the automatic FIRST review)
    //   pull_request               closed                  → cancel the PR's active tasks
    //   pull_request               synchronize | reopened  → nothing (re-review via @mention)
    //   issue_comment              created, body @<handle> → review task (a manual re-review)
    //   installation               created                 → register the installed repos as pending
    //   installation               deleted                 → disable the installation's repos
    //   installation_repositories  added | removed         → register pending / disable those repos
    //
    // Repos start **pending** and need admin approval before any review/index runs (Epic #75).
    // Everything else is persisted to `github_deliveries` only.
    if state.db.is_some() {
        match event.as_str() {
            "pull_request" => handle_pull_request(&state, &payload, &delivery_id).await,
            "issue_comment" => handle_issue_comment(&state, &payload, &delivery_id).await,
            "installation" => handle_installation(&state, &payload, &delivery_id).await,
            "installation_repositories" => {
                handle_installation_repositories(&state, &payload, &delivery_id).await
            }
            _ => {}
        }
    }

    (StatusCode::ACCEPTED, "accepted")
}

/// True when a comment body is addressed to the app — its first non-space text is `@<handle>`
/// (case-insensitive). A leading `@<handle>` is how a human asks for a re-review.
fn mentions_handle(body: &str, handle: &str) -> bool {
    let mention = format!("@{}", handle.to_ascii_lowercase());
    body.trim_start().to_ascii_lowercase().starts_with(&mention)
}

/// `pull_request` events. `opened` → the automatic first review. `closed` → cancel the PR's active
/// tasks (the reaper then stops their Jobs). `synchronize`/`reopened` do nothing — a re-review is
/// requested with an `@<handle>` comment ([`handle_issue_comment`]).
async fn handle_pull_request(
    state: &crate::AppState,
    payload: &serde_json::Value,
    delivery_id: &str,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    let action = payload["action"].as_str().unwrap_or_default();
    if !matches!(action, "opened" | "closed") {
        return;
    }
    let repo = &payload["repository"];
    let (Some(github_repo_id), Some(owner), Some(name), Some(default_branch), Some(pr_number)) = (
        repo["id"].as_i64(),
        repo["owner"]["login"].as_str(),
        repo["name"].as_str(),
        repo["default_branch"].as_str(),
        payload["pull_request"]["number"].as_i64(),
    ) else {
        tracing::warn!(
            delivery_id,
            "pull_request payload missing repo/number fields; skipping"
        );
        return;
    };
    let repository_id =
        match crate::db::upsert_repository(pool, github_repo_id, owner, name, default_branch).await
        {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(%error, delivery_id, "failed to upsert repository");
                return;
            }
        };

    match action {
        "opened" => {
            let Some(installation_id) = payload["installation"]["id"].as_i64() else {
                return;
            };
            // Approval gate (Epic #75): a repo must be admin-approved before any review runs.
            if !approved_or_skip(pool, repository_id, delivery_id, pr_number).await {
                return;
            }
            let pr = &payload["pull_request"];
            let task = crate::db::NewTask {
                repository_id,
                installation_id,
                github_delivery_id: delivery_id.to_string(),
                target_type: "pull_request".to_string(),
                target_id: pr_number,
                command_text: "review".to_string(),
                base_sha: pr["base"]["sha"].as_str().map(str::to_string),
                head_sha: pr["head"]["sha"].as_str().map(str::to_string),
                run_epoch: 0, // the automatic first review
            };
            create_review_task(state, pool, task, owner, name, delivery_id).await;
        }
        "closed" => {
            match crate::db::cancel_active_tasks_for_pr(pool, repository_id, pr_number).await {
                Ok(ids) if !ids.is_empty() => tracing::info!(
                    delivery_id,
                    pr = pr_number,
                    cancelled = ids.len(),
                    "PR closed; cancelled active tasks (reaper stops their Jobs)"
                ),
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(%error, delivery_id, pr = pr_number, "failed to cancel PR tasks")
                }
            }
        }
        _ => {}
    }
}

/// `issue_comment` on a PR whose body starts with `@<handle>` → a manual re-review. The comment
/// payload has no SHAs, so we fetch the PR's base/head via the API; the next `run_epoch` lets a fresh
/// task through the idempotency index even when the head SHA is unchanged.
async fn handle_issue_comment(
    state: &crate::AppState,
    payload: &serde_json::Value,
    delivery_id: &str,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    if payload["action"].as_str() != Some("created") {
        return;
    }
    // An issue is a PR only when it carries a `pull_request` object.
    if payload["issue"]["pull_request"].is_null() {
        return;
    }
    let body = payload["comment"]["body"].as_str().unwrap_or_default();
    if !mentions_handle(body, &state.app_handle) {
        return; // not addressed to us
    }

    let repo = &payload["repository"];
    let (
        Some(github_repo_id),
        Some(owner),
        Some(name),
        Some(default_branch),
        Some(installation_id),
        Some(pr_number),
    ) = (
        repo["id"].as_i64(),
        repo["owner"]["login"].as_str(),
        repo["name"].as_str(),
        repo["default_branch"].as_str(),
        payload["installation"]["id"].as_i64(),
        payload["issue"]["number"].as_i64(),
    )
    else {
        tracing::warn!(
            delivery_id,
            "issue_comment payload missing fields; skipping"
        );
        return;
    };

    let Some(app) = state.github.as_ref() else {
        tracing::warn!(
            delivery_id,
            "github app not configured; cannot fetch PR for re-review"
        );
        return;
    };
    let token = match app.installation_token(installation_id).await {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, delivery_id, "mint token for re-review failed");
            return;
        }
    };
    let (base_sha, head_sha) = match app.pull_request_shas(&token, owner, name, pr_number).await {
        Ok(shas) => shas,
        Err(error) => {
            tracing::error!(%error, delivery_id, pr = pr_number, "fetch PR SHAs failed");
            return;
        }
    };
    let repository_id =
        match crate::db::upsert_repository(pool, github_repo_id, owner, name, default_branch).await
        {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(%error, delivery_id, "failed to upsert repository");
                return;
            }
        };
    // Approval gate (Epic #75): even an explicit @mention can't run on an unapproved repo.
    if !approved_or_skip(pool, repository_id, delivery_id, pr_number).await {
        return;
    }
    let run_epoch = crate::db::next_run_epoch(
        pool,
        repository_id,
        "pull_request",
        pr_number,
        "review",
        head_sha.as_deref(),
    )
    .await
    .unwrap_or(0);
    let task = crate::db::NewTask {
        repository_id,
        installation_id,
        github_delivery_id: delivery_id.to_string(),
        target_type: "pull_request".to_string(),
        target_id: pr_number,
        command_text: "review".to_string(),
        base_sha,
        head_sha,
        run_epoch,
    };
    tracing::info!(delivery_id, pr = pr_number, "@mention re-review requested");
    create_review_task(state, pool, task, owner, name, delivery_id).await;
}

/// Insert a review task and, on a real insert, 👀 the PR (spawned so external GitHub calls can't
/// block the webhook's ~10s deadline). Shared by the auto-open and manual-mention paths.
async fn create_review_task(
    state: &crate::AppState,
    pool: &sqlx::PgPool,
    task: crate::db::NewTask,
    owner: &str,
    name: &str,
    delivery_id: &str,
) {
    let (pr, installation_id, run_epoch) = (task.target_id, task.installation_id, task.run_epoch);
    match crate::db::create_task(pool, &task).await {
        Ok(Some(task_id)) => {
            crate::metrics::task_created();
            tracing::info!(delivery_id, %task_id, pr, run_epoch, "created review task");
            let state = state.clone();
            let (owner, name) = (owner.to_string(), name.to_string());
            tokio::spawn(async move {
                react_seen(&state, &owner, &name, installation_id, pr).await;
            });
        }
        Ok(None) => tracing::info!(
            delivery_id,
            pr,
            run_epoch,
            "review task already exists; skipping (idempotent)"
        ),
        Err(error) => tracing::error!(%error, delivery_id, pr, "failed to create task"),
    }
}

/// The approval gate (Epic #75): returns `true` only when the repo is admin-approved. A
/// pending/disabled repo (or a query error — fail closed) logs and returns `false`, so no review/index
/// task is created. This is what stops the tool from running on repos nobody opted in.
async fn approved_or_skip(
    pool: &sqlx::PgPool,
    repository_id: i64,
    delivery_id: &str,
    pr: i64,
) -> bool {
    match crate::db::repository_approved(pool, repository_id).await {
        Ok(true) => true,
        Ok(false) => {
            tracing::info!(
                delivery_id,
                pr,
                repository_id,
                "repository not approved; skipping (awaiting admin approval)"
            );
            false
        }
        Err(error) => {
            tracing::error!(%error, delivery_id, repository_id, "approval check failed; skipping (fail closed)");
            false
        }
    }
}

/// `installation` events: `created` (the App was installed on an account) registers the selected
/// repos as **pending** approval; `deleted` (uninstalled) disables them. Repos default to pending so
/// nothing runs until an admin approves (Epic #75). The installation payload's repo objects carry no
/// `default_branch`; a placeholder is fine — the first PR webhook fills it in.
async fn handle_installation(
    state: &crate::AppState,
    payload: &serde_json::Value,
    delivery_id: &str,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    let action = payload["action"].as_str().unwrap_or_default();
    let repos = payload["repositories"].as_array();
    match action {
        "created" => register_pending(pool, repos, delivery_id).await,
        "deleted" => disable_repos(pool, repos, delivery_id).await,
        _ => {} // suspend/unsuspend/new_permissions_accepted → persisted only
    }
}

/// `installation_repositories` events: repos added to / removed from an existing installation.
/// Added → pending (await approval); removed → disabled.
async fn handle_installation_repositories(
    state: &crate::AppState,
    payload: &serde_json::Value,
    delivery_id: &str,
) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    register_pending(pool, payload["repositories_added"].as_array(), delivery_id).await;
    disable_repos(
        pool,
        payload["repositories_removed"].as_array(),
        delivery_id,
    )
    .await;
}

/// Register each repo (a webhook repo object: `id`, `full_name`) as pending approval, insert-only so
/// an already-approved repo is untouched.
async fn register_pending(
    pool: &sqlx::PgPool,
    repos: Option<&Vec<serde_json::Value>>,
    delivery_id: &str,
) {
    for repo in repos.into_iter().flatten() {
        let Some((github_repo_id, owner, name)) = repo_identity(repo) else {
            continue;
        };
        match crate::db::register_pending_repository(pool, github_repo_id, owner, name, "").await {
            Ok(true) => {
                tracing::info!(delivery_id, repo = %format!("{owner}/{name}"), "registered pending repository (awaiting approval)")
            }
            Ok(false) => {} // already known — leave its status as-is
            Err(error) => {
                tracing::error!(%error, delivery_id, "register pending repository failed")
            }
        }
    }
}

/// Mark each repo `disabled` (removed from the installation). Index-data purge is Milestone B.
async fn disable_repos(
    pool: &sqlx::PgPool,
    repos: Option<&Vec<serde_json::Value>>,
    delivery_id: &str,
) {
    for repo in repos.into_iter().flatten() {
        let Some(github_repo_id) = repo["id"].as_i64() else {
            continue;
        };
        if let Err(error) =
            crate::db::set_repository_status_by_github_id(pool, github_repo_id, "disabled").await
        {
            tracing::error!(%error, delivery_id, github_repo_id, "disable repository failed");
        } else {
            tracing::info!(
                delivery_id,
                github_repo_id,
                "repository disabled (removed from installation)"
            );
        }
    }
}

/// Extract `(github_repo_id, owner, name)` from a webhook repo object. The installation payload uses
/// `full_name` ("owner/name") rather than a nested owner object.
fn repo_identity(repo: &serde_json::Value) -> Option<(i64, &str, &str)> {
    let id = repo["id"].as_i64()?;
    let full_name = repo["full_name"].as_str()?;
    let (owner, name) = full_name.split_once('/')?;
    Some((id, owner, name))
}

/// Best-effort 👀 on the PR to acknowledge a review has started. Never fails the webhook: a missing
/// App, a token-mint error, or a GitHub hiccup is logged and ignored.
async fn react_seen(
    state: &crate::AppState,
    owner: &str,
    repo: &str,
    installation_id: i64,
    pr: i64,
) {
    if !state.review.reactions_enabled() {
        return;
    }
    let Some(app) = state.github.as_ref() else {
        return;
    };
    let token = match app.installation_token(installation_id).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(%error, pr, "react 👀: could not mint token (non-fatal)");
            return;
        }
    };
    if let Err(error) = app.add_reaction(&token, owner, repo, pr, "eyes").await {
        tracing::warn!(%error, pr, "react 👀 failed (non-fatal)");
    }
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Constant-time HMAC-SHA256 verification of the GitHub webhook signature.
/// An unset secret rejects everything (fail closed) rather than accepting all traffic.
fn verify_signature(secret: &[u8], body: &[u8], signature: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    use subtle::ConstantTimeEq;
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_must_lead_the_comment() {
        assert!(mentions_handle(
            "@lightbridge-assistant please review",
            "lightbridge-assistant"
        ));
        assert!(
            mentions_handle("  @Lightbridge-Assistant rerun", "lightbridge-assistant"),
            "leading space + case-insensitive"
        );
        // A mid-sentence mention is NOT a command (avoids re-running on casual references).
        assert!(!mentions_handle(
            "cc @lightbridge-assistant",
            "lightbridge-assistant"
        ));
        assert!(!mentions_handle(
            "just a normal comment",
            "lightbridge-assistant"
        ));
        assert!(!mentions_handle(
            "@someone-else go",
            "lightbridge-assistant"
        ));
    }

    #[test]
    fn repo_identity_parses_full_name() {
        let repo = serde_json::json!({ "id": 99, "full_name": "octo/Hello-World" });
        assert_eq!(repo_identity(&repo), Some((99, "octo", "Hello-World")));
        // Missing id / full_name, or a malformed full_name → None (skipped, not panicked).
        assert_eq!(repo_identity(&serde_json::json!({ "id": 1 })), None);
        assert_eq!(
            repo_identity(&serde_json::json!({ "id": 1, "full_name": "noslash" })),
            None
        );
    }

    #[test]
    fn rejects_when_secret_unset() {
        assert!(!verify_signature(b"", b"body", "sha256=anything"));
    }

    #[test]
    fn accepts_a_valid_signature() {
        let secret = b"it is a secret";
        let body = b"payload";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature(secret, body, &sig));
    }

    #[test]
    fn rejects_a_tampered_signature() {
        assert!(!verify_signature(b"secret", b"payload", "sha256=deadbeef"));
    }
}
