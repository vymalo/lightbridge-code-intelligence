//! Read API for tasks — the dashboard's data source (ADR-0016). Bearer-protected via the `Claims`
//! extractor (a valid OIDC access token is required).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use crate::jwt::Claims;
use crate::AppState;

const TASK_LIST_LIMIT: i64 = 100;

/// `GET /tasks` — most recent task runs first.
pub async fn list(_claims: Claims, State(state): State<AppState>) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::list_tasks(pool, TASK_LIST_LIMIT).await {
        Ok(tasks) => Json(tasks).into_response(),
        Err(error) => {
            tracing::error!(%error, "list tasks failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}

/// `GET /tasks/{id}` — a single task run, or 404.
pub async fn get(_claims: Claims, State(state): State<AppState>, Path(id): Path<Uuid>) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::get_task(pool, id).await {
        Ok(Some(task)) => Json(task).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, "get task failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}

/// `GET /tasks/{id}/review` — the persisted review for a run (summary + body + findings), or 404 when
/// none was recorded (older run, index task, or a review that never posted).
pub async fn get_review(
    _claims: Claims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::get_review(pool, id).await {
        Ok(Some(review)) => Json(review).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no review recorded").into_response(),
        Err(error) => {
            tracing::error!(%error, "get review failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}

/// `GET /repositories` — connected repositories + their run activity (the Repositories view).
pub async fn list_repositories(_claims: Claims, State(state): State<AppState>) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::list_repositories(pool, None).await {
        Ok(repos) => Json(repos).into_response(),
        Err(error) => {
            tracing::error!(%error, "list repositories failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}
