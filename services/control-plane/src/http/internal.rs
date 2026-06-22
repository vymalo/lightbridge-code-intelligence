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
    /// Run kind (ADR-0033): `review` (diff-scoped findings, the default) or `ask` (a conversational
    /// answer posted as a single reply comment). The runner branches on this.
    pub kind: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    /// Whether the repo already has a semantic index. The runner skips the full re-index on a review
    /// when this is true (reuses the base index + the PR diff) — ADR-0025.
    pub repo_indexed: bool,
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

    // A missing/failed index check is treated as "not indexed" (fail safe → the runner indexes),
    // so a transient DB hiccup degrades to the old always-index behavior rather than skipping.
    let repo_indexed = crate::db::repo_has_index(pool, context.repository_id)
        .await
        .unwrap_or(false);

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
        kind: context.kind,
        base_sha: context.base_sha,
        head_sha: context.head_sha,
        repo_indexed,
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

/// Body for `POST /internal/tasks/{id}/transcript` — the agent run transcript (ADR-0034).
#[derive(Debug, Deserialize)]
pub struct TranscriptSubmission {
    pub entries: Vec<crate::db::TranscriptInput>,
}

/// `POST /internal/tasks/{id}/transcript` — store the agent run transcript (ADR-0034). Replaces any
/// prior transcript for the task (a retry re-submits the whole thing).
pub async fn ingest_transcript(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(submission): Json<TranscriptSubmission>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::replace_transcript(pool, id, &submission.entries).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "storing transcript failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "store error").into_response()
        }
    }
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

    let nodes: Vec<crate::integrations::neo4j::GraphNode> = batch
        .nodes
        .into_iter()
        .map(|n| crate::integrations::neo4j::GraphNode {
            node_id: n.node_id,
            label: n.label,
            source_file: n.source_file,
            start_line: n.start_line,
        })
        .collect();
    let edges: Vec<crate::integrations::neo4j::GraphEdge> = batch
        .edges
        .into_iter()
        .map(|e| crate::integrations::neo4j::GraphEdge {
            source: e.source,
            target: e.target,
            relation: e.relation,
        })
        .collect();

    match crate::integrations::neo4j::upsert_graph(
        neo4j,
        repository_id,
        &batch.commit_sha,
        &nodes,
        &edges,
    )
    .await
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

/// Resolve a task's `(repository_id, commit_sha)` — the scope every retrieval query is pinned to.
/// `commit_sha` is the head SHA the index was built at (or the default branch). Returns `None` for
/// an unknown task. The caller never supplies the scope, so a task can only read its own repo.
async fn task_scope(pool: &sqlx::PgPool, id: Uuid) -> Result<Option<(i64, String)>, sqlx::Error> {
    Ok(crate::db::get_task_context(pool, id).await?.map(|ctx| {
        let commit = ctx.head_sha.unwrap_or(ctx.default_branch);
        (ctx.repository_id, commit)
    }))
}

/// Clamp a caller-supplied limit into a sane range (default 10, max 100).
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(10).clamp(1, 100)
}

/// Body for `POST /internal/tasks/{id}/search` — the query already embedded by the caller (the
/// vector MCP server embeds the text with the runner's embeddings key; the control plane holds none).
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `POST /internal/tasks/{id}/search` — semantic search over the task's pgvector index.
pub async fn search(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SearchRequest>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    if req.embedding.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty embedding").into_response();
    }
    let scope = match task_scope(pool, id).await {
        Ok(Some(scope)) => scope,
        Ok(None) => return (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "search scope lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
        }
    };
    let (repository_id, commit) = scope;
    match crate::db::search_code_chunks(
        pool,
        repository_id,
        &commit,
        &req.embedding,
        clamp_limit(req.limit),
    )
    .await
    {
        Ok(hits) => Json(hits).into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "semantic search failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "search error").into_response()
        }
    }
}

/// Body for `POST /internal/tasks/{id}/graph/query` — a small fixed op set over the Neo4j graph.
#[derive(Debug, Deserialize)]
pub struct GraphQueryRequest {
    /// `find_symbol` (needs `term`) or `get_callers` (needs `node_id`).
    pub op: String,
    #[serde(default)]
    pub term: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `POST /internal/tasks/{id}/graph/query` — structural queries over the task's Neo4j graph.
pub async fn graph_query(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<GraphQueryRequest>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    let Some(neo4j) = state.neo4j.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "neo4j not configured").into_response();
    };
    let scope = match task_scope(pool, id).await {
        Ok(Some(scope)) => scope,
        Ok(None) => return (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "graph-query scope lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
        }
    };
    let (repository_id, commit) = scope;
    let limit = clamp_limit(req.limit);

    let result = match req.op.as_str() {
        "find_symbol" => {
            let Some(term) = req.term.as_deref() else {
                return (StatusCode::BAD_REQUEST, "find_symbol requires `term`").into_response();
            };
            crate::integrations::neo4j::find_symbol(neo4j, repository_id, &commit, term, limit)
                .await
        }
        "get_callers" => {
            let Some(node_id) = req.node_id.as_deref() else {
                return (StatusCode::BAD_REQUEST, "get_callers requires `node_id`").into_response();
            };
            crate::integrations::neo4j::get_callers(neo4j, repository_id, &commit, node_id, limit)
                .await
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unsupported op {other:?} (expected: find_symbol | get_callers)"),
            )
                .into_response();
        }
    };

    match result {
        Ok(hits) => Json(hits).into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, op = %req.op, "graph query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "graph query error").into_response()
        }
    }
}

// ── ADR-0037 mediated write actions ─────────────────────────────────────────────────────────────
// The native agent calls these *during* its run; the control plane accumulates them and posts nothing
// until `finalize_review` flushes the buffer as one grouped review (+ a single consolidated reply).
// Per-call diff validation is done runner-side (it holds the diff); the flush re-validates here
// authoritatively via `crate::review::validate`.

/// Default summary for a run that produced no findings (and the empty-run backstop), so an
/// `@mention`-triggered review is never a silent hang (ADR-0037).
const DEFAULT_CLEAN_SUMMARY: &str = "No issues found — the change looks good.";

/// Body for `POST /internal/tasks/{id}/review/inline` (`add_review_comment`).
#[derive(Debug, Deserialize)]
pub struct InlineActionBody {
    pub file: String,
    pub line: i32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub suggestion: Option<String>,
    pub body: String,
}

/// Body for `POST /internal/tasks/{id}/review/comment` (`add_comment`) and
/// `POST /internal/tasks/{id}/review/summary` (`set_summary`).
#[derive(Debug, Deserialize)]
pub struct TextActionBody {
    pub body: String,
}

/// `POST /internal/tasks/{id}/review/inline` — buffer one inline finding (ADR-0037). Last write wins
/// per `(file, line)`; nothing is posted until [`finalize_review`].
pub async fn add_review_comment(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(a): Json<InlineActionBody>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::upsert_pending_inline(
        pool,
        id,
        &a.file,
        a.line,
        a.title.as_deref(),
        a.priority.as_deref(),
        a.category.as_deref(),
        a.suggestion.as_deref(),
        &a.body,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "buffering inline finding failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "buffer error").into_response()
        }
    }
}

/// `POST /internal/tasks/{id}/review/comment` — buffer one plain reply (`add_comment`, ADR-0037).
pub async fn add_review_reply(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(a): Json<TextActionBody>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::add_pending_comment(pool, id, &a.body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "buffering comment failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "buffer error").into_response()
        }
    }
}

/// `POST /internal/tasks/{id}/review/summary` — set the run's summary/verdict (`set_summary`).
pub async fn set_review_summary(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(a): Json<TextActionBody>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::upsert_pending_summary(pool, id, &a.body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "buffering summary failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "buffer error").into_response()
        }
    }
}

/// `POST /internal/tasks/{id}/review/finalize` — flush the accumulated buffer (ADR-0037). Posts the
/// inline findings + summary as **one grouped PR review** (re-validated against the diff here, the
/// authority), consolidates buffered replies into **one** thread comment, records the emergent run
/// kind, and clears the buffer. An empty run still posts a default "no issues found" review so an
/// `@mention` is never silent. The buffer is cleared at the end regardless, so a finished run can't
/// re-post on a stray retry.
pub async fn finalize_review(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    let Some(app) = state.github.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "github app not configured").into_response();
    };
    let context = match crate::db::get_task_context(pool, id).await {
        Ok(Some(c)) => c,
        Ok(None) => return (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "load task for finalize failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
        }
    };
    let pending = match crate::db::load_pending_review(pool, id).await {
        Ok(p) => p,
        Err(error) => {
            tracing::error!(%error, task_id = %id, "load pending buffer failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response();
        }
    };
    let token = match app.installation_token(context.installation_id).await {
        Ok(t) => t,
        Err(error) => {
            tracing::error!(%error, task_id = %id, "mint installation token failed");
            return (StatusCode::BAD_GATEWAY, "could not mint installation token").into_response();
        }
    };
    let target = ReviewTarget {
        token: &token,
        owner: &context.owner,
        repo: &context.name,
        pr: context.target_id,
    };

    // 1) Buffered replies → a single consolidated thread comment (works on a PR or an issue). On a
    // successful post we drop the comment rows immediately, so a re-finalize (e.g. a retried delivery
    // after a later step failed) never double-posts this reply.
    let mut posted_reply = false;
    if !pending.comments.is_empty() {
        let body = crate::review::render_answer_body(&pending.comments.join("\n\n---\n\n"));
        match app
            .create_issue_comment(
                &token,
                &context.owner,
                &context.name,
                context.target_id,
                &body,
            )
            .await
        {
            Ok(_) => {
                posted_reply = true;
                let _ = crate::db::clear_pending_action(pool, id, "comment").await;
            }
            Err(error) => {
                tracing::warn!(%error, task_id = %id, "posting consolidated reply failed (non-fatal)")
            }
        }
    }

    // 2) Inline findings + summary → one grouped PR review (PR targets only). Also covers the
    // empty-run backstop (post a default clean review) and a summary-only verdict.
    let has_inline = !pending.inline.is_empty();
    let post_pr_review = context.target_type == "pull_request"
        && (has_inline || pending.summary.is_some() || pending.is_empty());
    let mut posted_review = false;
    let mut review_failed = false;
    if post_pr_review {
        let pr = context.target_id;
        let findings: Vec<crate::review::Finding> = pending
            .inline
            .iter()
            .map(|pi| crate::review::Finding {
                file: pi.file.clone(),
                line: pi.line.max(0) as u32,
                priority: pi.priority.clone(),
                category: pi.category.clone(),
                severity: None,
                title: pi.title.clone().unwrap_or_default(),
                body: pi.body.clone(),
                suggestion: pi.suggestion.clone(),
                resources: Vec::new(),
            })
            .collect();
        // A present-but-blank summary falls back to the default, so the verdict is never empty.
        let summary = pending
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_CLEAN_SUMMARY)
            .to_string();

        let commentable: std::collections::HashMap<String, std::collections::BTreeSet<u32>> =
            match app
                .list_pr_files(&token, &context.owner, &context.name, pr)
                .await
            {
                Ok(files) => files
                    .into_iter()
                    .filter_map(|f| {
                        f.patch
                            .map(|p| (f.filename, crate::review::commentable_lines(&p)))
                    })
                    .collect(),
                Err(error) => {
                    tracing::error!(%error, task_id = %id, "fetching PR files failed");
                    return (StatusCode::BAD_GATEWAY, "could not fetch PR files").into_response();
                }
            };

        let in_scope = |f: &crate::review::Finding| {
            let normalized = f.file.replace('\\', "/");
            let trimmed = normalized.trim_start_matches("./").trim_start_matches('/');
            commentable.contains_key(trimmed) || commentable.contains_key(&f.file)
        };
        let label_has_findings = findings.iter().any(in_scope);
        let label_has_error = findings.iter().any(|f| f.priority() == "P0" && in_scope(f));
        let findings_json = serde_json::to_value(&findings).unwrap_or_default();

        let validated = crate::review::validate(findings, &commentable);
        let body =
            crate::review::render_body(&summary, &validated.deferred, &validated.out_of_scope);
        let comments: Vec<crate::integrations::github::ReviewComment> = validated
            .inline
            .iter()
            .map(|c| crate::integrations::github::ReviewComment {
                path: c.path.clone(),
                line: c.line,
                side: "RIGHT",
                body: c.body.clone(),
            })
            .collect();
        let (inline_n, deferred_n, out_of_scope_n) = (
            comments.len(),
            validated.deferred.len(),
            validated.out_of_scope.len(),
        );

        match app
            .create_pr_review(&token, &context.owner, &context.name, pr, &body, &comments)
            .await
        {
            Ok(review_url) => {
                posted_review = true;
                tracing::info!(task_id = %id, inline = inline_n, deferred = deferred_n, out_of_scope = out_of_scope_n, "review flushed");
                if let Err(error) = crate::db::upsert_review(
                    pool,
                    id,
                    &summary,
                    &body,
                    inline_n as i32,
                    deferred_n as i32,
                    out_of_scope_n as i32,
                    &findings_json,
                    review_url.as_deref(),
                )
                .await
                {
                    tracing::warn!(%error, task_id = %id, "persisting review copy failed (non-fatal)");
                }
                react(app, &state.review, &target, "hooray").await;
                add_review_labels(
                    app,
                    &state.review,
                    &target,
                    label_has_findings,
                    label_has_error,
                )
                .await;
                // Drop the inline + summary rows now that the review is on GitHub, so a re-finalize
                // doesn't post a duplicate review.
                let _ = crate::db::clear_pending_action(pool, id, "inline").await;
                let _ = crate::db::clear_pending_action(pool, id, "summary").await;
            }
            Err(error) => {
                tracing::error!(%error, task_id = %id, "flushing PR review failed");
                react(app, &state.review, &target, "confused").await;
                // Leave the inline/summary rows intact so a re-finalize can post the review; don't
                // bail early, so the kind is still recorded and any posted reply isn't lost.
                review_failed = true;
            }
        }
    }

    // 3) Record the emergent run kind (ADR-0037). The buffer was cleared per-part as each post
    // succeeded; any rows left belong to a part that failed and a re-finalize will retry.
    let kind = match (has_inline, posted_reply) {
        (true, true) => "mixed",
        (true, false) => "review",
        (false, true) => "ask",
        (false, false) => "review", // summary-only or empty → a (clean) review
    };
    let _ = crate::db::set_task_kind(pool, id, kind).await;

    if review_failed {
        return (StatusCode::BAD_GATEWAY, "could not post review").into_response();
    }
    Json(serde_json::json!({ "kind": kind, "review": posted_review, "reply": posted_reply }))
        .into_response()
}

/// Where a review reaction/label is applied: the minted token + the PR coordinates.
struct ReviewTarget<'a> {
    token: &'a str,
    owner: &'a str,
    repo: &'a str,
    pr: i64,
}

/// Best-effort PR reaction for review lifecycle feedback (👀 started / 🎉 done / 😕 errored). A
/// disabled toggle or any GitHub error is a no-op — review delivery never fails over a reaction.
async fn react(
    app: &crate::integrations::github::GithubApp,
    review: &crate::config::ReviewSection,
    target: &ReviewTarget<'_>,
    content: &str,
) {
    if !review.reactions_enabled() {
        return;
    }
    if let Err(error) = app
        .add_reaction(target.token, target.owner, target.repo, target.pr, content)
        .await
    {
        tracing::warn!(%error, pr = target.pr, content, "review reaction failed (non-fatal)");
    }
}

/// Best-effort outcome labels from config: `label_reviewed` always (when set), `label_findings` when
/// the review had in-scope findings, `label_error` when any were `error`-severity.
async fn add_review_labels(
    app: &crate::integrations::github::GithubApp,
    review: &crate::config::ReviewSection,
    target: &ReviewTarget<'_>,
    has_findings: bool,
    has_error: bool,
) {
    let mut labels = Vec::new();
    if let Some(label) = &review.label_reviewed {
        labels.push(label.clone());
    }
    if has_findings {
        if let Some(label) = &review.label_findings {
            labels.push(label.clone());
        }
    }
    if has_error {
        if let Some(label) = &review.label_error {
            labels.push(label.clone());
        }
    }
    if !labels.is_empty() {
        if let Err(error) = app
            .add_labels(target.token, target.owner, target.repo, target.pr, &labels)
            .await
        {
            tracing::warn!(%error, pr = target.pr, "adding review labels failed (non-fatal)");
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
        Ok(true) => {
            // ADR-0037 idempotency: a runner (re)starting its task clears any buffer left by a prior
            // attempt, so a retry accumulates from empty rather than appending to a partial review.
            if update.status == "running" {
                if let Err(error) = crate::db::clear_pending_review(pool, id).await {
                    tracing::warn!(%error, task_id = %id, "clearing pending buffer on (re)start failed (non-fatal)");
                }
            }
            // A terminal failure gets 😕 on the PR (best-effort). Success is acknowledged by the
            // review post (🎉) in `finalize_review`, so we don't double-react here.
            if matches!(update.status.as_str(), "failed" | "timed_out") {
                let state = state.clone();
                let pool = pool.clone();
                tokio::spawn(async move {
                    react_failure(&state, &pool, id).await;
                });
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "set task status failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "update error").into_response()
        }
    }
}

/// The current task status, for the runner's self-cancel poll.
#[derive(Debug, Serialize)]
pub struct TaskStatusResponse {
    pub status: String,
}

/// `GET /internal/tasks/{id}/status` — the task's current status, so the runner can stop promptly
/// when its task is cancelled (e.g. its PR closed) even if the reaper that would delete the Job is
/// down. Lightweight: no token mint. A missing task is `404` — the runner treats that as "stop" too.
pub async fn get_status(
    _auth: RunnerAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    match crate::db::get_task_status(pool, id).await {
        Ok(Some(status)) => Json(TaskStatusResponse { status }).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "task not found").into_response(),
        Err(error) => {
            tracing::error!(%error, task_id = %id, "get task status failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "query error").into_response()
        }
    }
}

/// Best-effort 😕 on the PR when a review task fails. Loads the task's PR context + mints a token;
/// any error (no App, non-PR task, GitHub hiccup) is logged and ignored.
async fn react_failure(state: &AppState, pool: &sqlx::PgPool, id: Uuid) {
    if !state.review.reactions_enabled() {
        return;
    }
    let Some(app) = state.github.as_ref() else {
        return;
    };
    let context = match crate::db::get_task_context(pool, id).await {
        Ok(Some(context)) if context.target_type == "pull_request" => context,
        _ => return,
    };
    match app.installation_token(context.installation_id).await {
        Ok(token) => {
            let target = ReviewTarget {
                token: &token,
                owner: &context.owner,
                repo: &context.name,
                pr: context.target_id,
            };
            react(app, &state.review, &target, "confused").await;
        }
        Err(error) => tracing::warn!(%error, task_id = %id, "react 😕: could not mint token"),
    }
}
