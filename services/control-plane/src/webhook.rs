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
        return (StatusCode::ACCEPTED, "duplicate delivery");
    }

    tracing::info!(delivery_id, event, "accepted webhook");

    // Route actionable events to a task. For now: a pull_request opened/synchronize/reopened
    // becomes a review task. Other events are persisted only. (issue_comment commands: follow-up.)
    if let Some(pool) = &state.db {
        if event == "pull_request" {
            create_pr_task(pool, &payload, &delivery_id).await;
        }
    }

    (StatusCode::ACCEPTED, "accepted")
}

/// Create a review task from a `pull_request` webhook (best-effort; logs and skips on malformed
/// payloads — the delivery is already persisted, so nothing is lost).
async fn create_pr_task(pool: &sqlx::PgPool, payload: &serde_json::Value, delivery_id: &str) {
    let action = payload["action"].as_str().unwrap_or_default();
    if !matches!(action, "opened" | "synchronize" | "reopened") {
        return;
    }
    let repo = &payload["repository"];
    let (
        Some(github_repo_id),
        Some(owner),
        Some(name),
        Some(default_branch),
        Some(installation_id),
    ) = (
        repo["id"].as_i64(),
        repo["owner"]["login"].as_str(),
        repo["name"].as_str(),
        repo["default_branch"].as_str(),
        payload["installation"]["id"].as_i64(),
    )
    else {
        tracing::warn!(
            delivery_id,
            "pull_request payload missing fields; skipping task"
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

    let pr = &payload["pull_request"];
    let task = crate::db::NewTask {
        repository_id,
        installation_id,
        github_delivery_id: delivery_id.to_string(),
        target_type: "pull_request".to_string(),
        target_id: pr["number"].as_i64().unwrap_or_default(),
        command_text: "review".to_string(),
        base_sha: pr["base"]["sha"].as_str().map(str::to_string),
        head_sha: pr["head"]["sha"].as_str().map(str::to_string),
    };
    match crate::db::create_task(pool, &task).await {
        Ok(task_id) => {
            tracing::info!(delivery_id, %task_id, pr = task.target_id, "created review task")
        }
        Err(error) => tracing::error!(%error, delivery_id, "failed to create task"),
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
