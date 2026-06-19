//! Internal runner API — the control-plane side of the runner↔control-plane contract (ADR-0017).
//!
//! The dispatcher launches one Kubernetes Job per task (ADR-0004); that Job runs the agent runner,
//! which has no GitHub App key of its own. Per the trust boundary (ADR-0002) the runner calls back
//! here to (a) fetch its task context plus a freshly-minted, short-lived installation token, and
//! (b) report status transitions. These routes are **not** OIDC-protected (the caller is a pod, not
//! a user): they authenticate with a shared bearer (`AGENT_RUNNER_TOKEN`) the control plane injects
//! into the Job. Absent that token in this process, the routes fail closed (503) — never open.

use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::AppState;

/// Authenticates a runner request by comparing its `Authorization: Bearer` token against the
/// configured `AGENT_RUNNER_TOKEN`, in constant time. A unit extractor: presence of the value is
/// the whole proof, so there is nothing to carry.
pub struct RunnerAuth;

/// Rejections for the internal API. 401 for a bad/missing token; 503 when the shared secret is not
/// configured in this process (so the surface is closed rather than unauthenticated).
pub enum RunnerAuthError {
    MissingToken,
    InvalidToken,
    Disabled,
}

impl IntoResponse for RunnerAuthError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            RunnerAuthError::MissingToken => (StatusCode::UNAUTHORIZED, "missing bearer token"),
            RunnerAuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "invalid runner token"),
            RunnerAuthError::Disabled => {
                (StatusCode::SERVICE_UNAVAILABLE, "runner api not configured")
            }
        };
        (status, msg).into_response()
    }
}

impl FromRequestParts<AppState> for RunnerAuth {
    type Rejection = RunnerAuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, RunnerAuthError> {
        let expected = state
            .runner_token
            .as_ref()
            .ok_or(RunnerAuthError::Disabled)?;
        let presented = bearer_token(parts).ok_or(RunnerAuthError::MissingToken)?;
        // Constant-time compare so a wrong token can't be recovered byte-by-byte via timing.
        if presented.as_bytes().ct_eq(expected.as_bytes()).into() {
            Ok(RunnerAuth)
        } else {
            Err(RunnerAuthError::InvalidToken)
        }
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

/// The runner's view of a task: where the code is, how to fetch it, and what to do. `token` is a
/// short-lived installation access token (~1h) minted just-in-time; `clone_url` is the plain HTTPS
/// remote (the runner composes the authenticated URL with the token, so the token isn't baked into
/// a value it might log).
#[derive(Debug, Serialize)]
pub struct TaskContextResponse {
    pub task_id: Uuid,
    pub repository_id: i64,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub token: String,
    pub target_type: String,
    pub target_id: i64,
    pub command: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
}

/// `GET /internal/tasks/{id}` — task context + a freshly-minted installation token for the runner.
pub async fn get_context(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    let Some(app) = state.github.as_ref() else {
        // Without the App key we cannot mint a token, so the runner could not clone — fail closed.
        return (StatusCode::SERVICE_UNAVAILABLE, "github app not configured").into_response();
    };

    let context = match crate::db::get_task_context(pool, id).await {
        Ok(Some(context)) => context,
        Ok(None) => return (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "load task context failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
        }
    };

    let token = match app.installation_token(context.installation_id).await {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, task_id = %id, "mint installation token failed");
            return (StatusCode::BAD_GATEWAY, "could not mint installation token").into_response();
        }
    };

    Json(TaskContextResponse {
        task_id: context.id,
        repository_id: context.repository_id,
        clone_url: format!("https://github.com/{}/{}.git", context.owner, context.name),
        owner: context.owner,
        name: context.name,
        default_branch: context.default_branch,
        token,
        target_type: context.target_type,
        target_id: context.target_id,
        command: context.command_text,
        base_sha: context.base_sha,
        head_sha: context.head_sha,
    })
    .into_response()
}

/// One chunk submitted by the indexer runner.
#[derive(Debug, Deserialize)]
pub struct ChunkInput {
    pub file_path: String,
    pub language: String,
    pub chunk_type: String,
    pub symbol_name: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
    pub content: String,
    pub embedding: Vec<f32>,
}

/// Body for `POST /internal/tasks/{id}/chunks`.
#[derive(Debug, Deserialize)]
pub struct ChunkBatch {
    pub commit_sha: String,
    pub chunks: Vec<ChunkInput>,
}

/// `POST /internal/tasks/{id}/chunks` — ingest indexed code chunks from the runner.
///
/// The runner submits chunks in batches as it processes files; the control plane writes them to
/// `code_chunks` (pgvector). The task's `repository_id` is read from the DB — the runner cannot
/// supply it (trust boundary, ADR-0002).
pub async fn ingest_chunks(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(batch): Json<ChunkBatch>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };

    let repository_id: Option<i64> =
        match sqlx::query_scalar("SELECT repository_id FROM tasks WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
        {
            Ok(row) => row,
            Err(error) => {
                tracing::error!(%error, task_id = %id, "load task for chunk ingest failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
            }
        };

    let Some(repository_id) = repository_id else {
        return (StatusCode::NOT_FOUND, "task not found").into_response();
    };

    if batch.chunks.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let chunks: Vec<crate::db::CodeChunk> = batch
        .chunks
        .into_iter()
        .map(|c| crate::db::CodeChunk {
            file_path: c.file_path,
            language: c.language,
            chunk_type: c.chunk_type,
            symbol_name: c.symbol_name,
            start_line: c.start_line,
            end_line: c.end_line,
            content: c.content,
            embedding: c.embedding,
        })
        .collect();

    match crate::db::upsert_code_chunks(pool, repository_id, &batch.commit_sha, &chunks).await {
        Ok(count) => {
            tracing::info!(task_id = %id, chunk_count = count, "chunks ingested");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => {
            tracing::error!(%error, task_id = %id, "chunk upsert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "upsert error").into_response()
        }
    }
}

/// One structural-graph node submitted by the runner (a Graphify `graph.json` node).
#[derive(Debug, Deserialize)]
pub struct GraphNodeInput {
    pub node_id: String,
    pub label: String,
    pub source_file: String,
    pub start_line: i64,
}

/// One directed edge (`contains` / `method` / `calls` / …).
#[derive(Debug, Deserialize)]
pub struct GraphEdgeInput {
    pub source: String,
    pub target: String,
    pub relation: String,
}

/// Body for `POST /internal/tasks/{id}/graph`.
#[derive(Debug, Deserialize)]
pub struct GraphBatch {
    pub commit_sha: String,
    pub nodes: Vec<GraphNodeInput>,
    pub edges: Vec<GraphEdgeInput>,
}

/// `POST /internal/tasks/{id}/graph` — ingest the structural code graph (Graphify → Neo4j, ADR-0019).
///
/// The runner spawns Graphify, reads its `graph.json`, and POSTs nodes+edges here; the control plane
/// writes them to Neo4j. `repository_id` is read from the DB, not trusted from the caller (ADR-0002).
pub async fn ingest_graph(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(batch): Json<GraphBatch>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    let Some(neo4j) = state.neo4j.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "neo4j not configured").into_response();
    };

    let repository_id: Option<i64> =
        match sqlx::query_scalar("SELECT repository_id FROM tasks WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
        {
            Ok(row) => row,
            Err(error) => {
                tracing::error!(%error, task_id = %id, "load task for graph ingest failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
            }
        };

    let Some(repository_id) = repository_id else {
        return (StatusCode::NOT_FOUND, "task not found").into_response();
    };

    if batch.nodes.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let nodes: Vec<crate::neo4j::GraphNode> = batch
        .nodes
        .into_iter()
        .map(|n| crate::neo4j::GraphNode {
            node_id: n.node_id,
            label: n.label,
            source_file: n.source_file,
            start_line: n.start_line,
        })
        .collect();
    let edges: Vec<crate::neo4j::GraphEdge> = batch
        .edges
        .into_iter()
        .map(|e| crate::neo4j::GraphEdge {
            source: e.source,
            target: e.target,
            relation: e.relation,
        })
        .collect();

    match crate::neo4j::upsert_graph(neo4j, repository_id, &batch.commit_sha, &nodes, &edges).await
    {
        Ok((n, e)) => {
            tracing::info!(task_id = %id, nodes = n, edges = e, "graph ingested");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(error) => {
            tracing::error!(%error, task_id = %id, "graph upsert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "upsert error").into_response()
        }
    }
}

/// The runner's status report. `detail` is optional free text for diagnostics (not persisted yet).
#[derive(Debug, Deserialize)]
pub struct StatusUpdate {
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// `POST /internal/tasks/{id}/status` — apply a runner-reported status transition.
pub async fn set_status(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(update): Json<StatusUpdate>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    if !crate::db::is_runner_reportable_status(&update.status) {
        return (StatusCode::BAD_REQUEST, "unsupported status").into_response();
    }
    if let Some(detail) = &update.detail {
        tracing::info!(task_id = %id, status = %update.status, detail, "runner status report");
    }
    match crate::db::set_task_status(pool, id, &update.status).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "set task status failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "update error").into_response()
        }
    }
}
