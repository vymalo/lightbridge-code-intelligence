//! Admin API for the approval gate (Epic #75, Milestone A).
//!
//! The GitHub App can be installed on any org/repo, but a repository is **not** indexed or reviewed
//! until an admin approves it (so nobody can point the tool at arbitrary private repos). These
//! endpoints — gated by the [`Admin`] extractor (a valid OIDC token carrying the configured admin
//! realm role) — let an admin see the pending queue and approve/deny repos.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::jwt::Admin;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct RepoListQuery {
    /// Optional approval-status filter, e.g. `?status=pending` for the approval queue.
    pub status: Option<String>,
}

/// `GET /admin/repositories[?status=pending]` — repositories for the admin console; filter by status
/// to show the approval queue.
pub async fn list_repositories(
    _admin: Admin,
    State(state): State<AppState>,
    Query(query): Query<RepoListQuery>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::list_repositories(pool, query.status.as_deref()).await {
        Ok(repos) => Json(repos).into_response(),
        Err(error) => {
            tracing::error!(%error, "admin list repositories failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}

/// `POST /admin/repositories/{id}/approve` — opt a repository in. Future PRs (and a base index, once
/// Milestone B lands) may then run. Records the approving admin's identity for audit.
pub async fn approve(admin: Admin, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    set_status(admin, state, id, "approved").await
}

/// `POST /admin/repositories/{id}/deny` — keep a repository out of scope (sets `disabled`). Index
/// data purge on denial is Milestone B; this just closes the gate.
pub async fn deny(admin: Admin, State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    set_status(admin, state, id, "disabled").await
}

/// Shared by approve/deny. Takes the already-extracted inner types (not Axum extractor wrappers)
/// since it's a plain helper, not a handler.
async fn set_status(admin: Admin, state: AppState, id: i64, status: &str) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    let by = admin.0.identity();
    match crate::db::set_repository_status_by_id(pool, id, status, Some(by)).await {
        Ok(Some(repo)) => {
            tracing::info!(
                repo_id = id,
                status,
                admin = by,
                "admin set repository status"
            );
            // Denial removes the repo from scope → purge its index data (Epic #75, Milestone B).
            if status == "disabled" {
                crate::lifecycle::spawn_purge(&state, id);
            }
            Json(repo).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "repository not found").into_response(),
        Err(error) => {
            tracing::error!(%error, repo_id = id, status, "admin set repository status failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}
