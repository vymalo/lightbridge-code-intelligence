//! The agent's tool surface (ADR-0026 + ADR-0037) and the in-process dispatcher that runs each call.
//!
//! These are the same capabilities the standalone MCP servers expose, but invoked directly against
//! the control-plane client instead of over stdio MCP — the MCP servers were already thin proxies to
//! the control-plane retrieval API, so the review agent needs no subprocess:
//!
//! - **Retrieval** (read-only, the model investigates with these): `vector_semantic_search`,
//!   `graph_find_symbol`, `graph_get_callers`.
//! - **Write actions** (ADR-0037 — the agent *acts* as it goes; the control plane mediates every
//!   write and posts nothing until finalize): `add_review_comment` (an inline finding),
//!   `add_comment` (a plain reply), `finish` (set the verdict + end the run).
//! - **Control**: `report_progress`, `abort`.
//!
//! A tool/argument error is returned to the model as text (so it can retry/rephrase), never as a
//! loop-killing error — the same recovery property the MCP servers had.

use serde::Deserialize;
use uuid::Uuid;

use super::chat::{ToolCall, ToolDef};
use crate::bootstrap::client::ControlPlaneClient;
use crate::indexer::embeddings::EmbeddingsClient;

// The retrieval tools keep the `lightbridge_`-prefixed names the MCP servers used, so a reviewer
// prompt that references them by name stays accurate for the native agent too.
pub const VECTOR_SEMANTIC_SEARCH: &str = "lightbridge_vector_semantic_search";
pub const GRAPH_FIND_SYMBOL: &str = "lightbridge_graph_find_symbol";
pub const GRAPH_GET_CALLERS: &str = "lightbridge_graph_get_callers";
pub const ADD_REVIEW_COMMENT: &str = "add_review_comment";
pub const ADD_COMMENT: &str = "add_comment";
pub const FINISH: &str = "finish";
pub const REPORT_PROGRESS: &str = "report_progress";
pub const ABORT: &str = "abort";

const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 100;

/// What the loop should do after a tool call.
#[derive(Debug)]
pub enum ToolOutcome {
    /// A result string to feed back to the model as a `tool` message; the loop continues.
    Continue(String),
    /// The model called `finish` — the run is done; the control plane flushes the buffered actions.
    Finish,
    /// The model called `abort` — it can't produce a useful result. Recorded, not a crash.
    Abort(String),
}

#[derive(Debug, Deserialize)]
struct SemanticSearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct FindSymbolArgs {
    term: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GetCallersArgs {
    node_id: String,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AddReviewCommentArgs {
    file: String,
    line: i32,
    title: String,
    priority: String,
    category: String,
    body: String,
    #[serde(default)]
    suggestion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TextArgs {
    body: String,
}

#[derive(Debug, Deserialize)]
struct FinishArgs {
    summary: String,
}

#[derive(Debug, Deserialize)]
struct NoteArgs {
    note: String,
}

#[derive(Debug, Deserialize)]
struct AbortArgs {
    reason: String,
}

/// The read-only retrieval tools the model investigates with.
fn retrieval_tool_defs() -> Vec<ToolDef> {
    let limit_schema = serde_json::json!({
        "type": "integer",
        "description": "Maximum number of results (default 10, max 100)."
    });
    vec![
        ToolDef::function(
            VECTOR_SEMANTIC_SEARCH,
            "Semantic search over the repository's indexed code by meaning (pgvector). Returns the \
             most similar code chunks with file path, line range, and score.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language or code query." },
                    "limit": limit_schema,
                },
                "required": ["query"],
            }),
        ),
        ToolDef::function(
            GRAPH_FIND_SYMBOL,
            "Find symbols (functions, classes, methods) by name, node id, or file-path substring. \
             Returns matching nodes with their node id, label, and location.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "term": { "type": "string", "description": "Symbol name / node id / file path substring (case-insensitive)." },
                    "limit": limit_schema,
                },
                "required": ["term"],
            }),
        ),
        ToolDef::function(
            GRAPH_GET_CALLERS,
            "Return the symbols that call a given symbol (reverse call graph). Pass a node id from \
             graph_find_symbol.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "node_id": { "type": "string", "description": "Node id of the target symbol (from graph_find_symbol)." },
                    "limit": limit_schema,
                },
                "required": ["node_id"],
            }),
        ),
    ]
}

/// The `report_progress` + `abort` control tools.
fn aux_control_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef::function(
            REPORT_PROGRESS,
            "Optionally report a short progress note for observability. Does not affect the result.",
            serde_json::json!({
                "type": "object",
                "properties": { "note": { "type": "string" } },
                "required": ["note"],
            }),
        ),
        ToolDef::function(
            ABORT,
            "Abort when you cannot produce a useful result (e.g. the diff is unreadable). Recorded \
             as a clean abort, not a crash. Nothing you buffered is posted.",
            serde_json::json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"],
            }),
        ),
    ]
}

/// The full tool surface (ADR-0037), in a stable order: retrieval + write actions + control.
pub fn tool_defs() -> Vec<ToolDef> {
    let mut defs = retrieval_tool_defs();
    defs.push(ToolDef::function(
        ADD_REVIEW_COMMENT,
        "Record one inline review finding on a line the diff adds or changes. Call once per finding \
         as you go; nothing posts until `finish`. Re-recording the same (file, line) refines it.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "description": "Path from repo root." },
                "line": { "type": "integer", "description": "A line this diff adds or changes." },
                "title": { "type": "string", "description": "Short (≤ ~8 words)." },
                "priority": { "type": "string", "enum": ["P0", "P1", "P2"], "description": "P0 = must fix (bug/security/data-loss), P1 = should fix, P2 = minor/nit." },
                "category": { "type": "string", "enum": ["security", "correctness", "quality", "style", "performance"], "description": "The dimension this finding is about." },
                "body": { "type": "string", "description": "Why it matters." },
                "suggestion": { "type": "string", "description": "Optional exact replacement source for `line` (no diff markers)." },
            },
            "required": ["file", "line", "title", "priority", "category", "body"],
        }),
    ));
    defs.push(ToolDef::function(
        ADD_COMMENT,
        "Post a plain reply on the thread (GitHub-flavored Markdown) — for answering a question or a \
         general remark, not pinned to a diff line. Multiple calls are consolidated into one reply.",
        serde_json::json!({
            "type": "object",
            "properties": { "body": { "type": "string", "description": "Markdown reply body." } },
            "required": ["body"],
        }),
    ));
    defs.push(ToolDef::function(
        FINISH,
        "Finish the run: record your overall verdict/summary and post everything you buffered. Call \
         exactly once when done — investigate and record findings/replies first.",
        serde_json::json!({
            "type": "object",
            "properties": { "summary": { "type": "string", "description": "1–3 sentence overall verdict: does the change do what it intends, and is it correct and safe?" } },
            "required": ["summary"],
        }),
    ));
    defs.extend(aux_control_tool_defs());
    defs
}

/// Runs tool calls against the control-plane API. Holds only borrowed clients + the task id.
pub struct Tools<'a> {
    pub client: &'a ControlPlaneClient,
    pub embedder: &'a EmbeddingsClient,
    pub task_id: Uuid,
}

impl Tools<'_> {
    /// Execute one tool call and say what the loop should do next. Tool/argument errors come back as
    /// [`ToolOutcome::Continue`] text so the model can recover rather than aborting the run.
    pub async fn dispatch(&self, call: &ToolCall) -> ToolOutcome {
        let name = call.function.name.as_str();
        let args = &call.function.arguments;
        match name {
            VECTOR_SEMANTIC_SEARCH => match parse::<SemanticSearchArgs>(args) {
                Ok(a) => self.vector_search(&a.query, clamp_limit(a.limit)).await,
                Err(e) => ToolOutcome::Continue(e),
            },
            GRAPH_FIND_SYMBOL => match parse::<FindSymbolArgs>(args) {
                Ok(a) => {
                    let r = self
                        .client
                        .graph_find_symbol(self.task_id, &a.term, clamp_limit(a.limit))
                        .await;
                    ToolOutcome::Continue(render(name, r))
                }
                Err(e) => ToolOutcome::Continue(e),
            },
            GRAPH_GET_CALLERS => match parse::<GetCallersArgs>(args) {
                Ok(a) => {
                    let r = self
                        .client
                        .graph_get_callers(self.task_id, &a.node_id, clamp_limit(a.limit))
                        .await;
                    ToolOutcome::Continue(render(name, r))
                }
                Err(e) => ToolOutcome::Continue(e),
            },
            ADD_REVIEW_COMMENT => match parse::<AddReviewCommentArgs>(args) {
                Ok(a) => match self
                    .client
                    .add_review_comment(
                        self.task_id,
                        &a.file,
                        a.line,
                        Some(&a.title),
                        Some(&a.priority),
                        Some(&a.category),
                        a.suggestion.as_deref(),
                        &a.body,
                    )
                    .await
                {
                    Ok(()) => {
                        ToolOutcome::Continue(format!("recorded finding at {}:{}", a.file, a.line))
                    }
                    Err(e) => {
                        ToolOutcome::Continue(format!("error: could not record finding: {e:#}"))
                    }
                },
                Err(e) => ToolOutcome::Continue(format!(
                    "{e} Expected JSON like {{\"file\": \"path\", \"line\": 42, \"title\": \"…\", \
                     \"priority\": \"P0\", \"category\": \"security\", \"body\": \"…\", \
                     \"suggestion\": \"optional\"}}. priority is P0|P1|P2; category is \
                     security|correctness|quality|style|performance."
                )),
            },
            ADD_COMMENT => match parse::<TextArgs>(args) {
                Ok(a) => match self.client.add_review_reply(self.task_id, &a.body).await {
                    Ok(()) => ToolOutcome::Continue("comment recorded".to_string()),
                    Err(e) => {
                        ToolOutcome::Continue(format!("error: could not record comment: {e:#}"))
                    }
                },
                Err(e) => ToolOutcome::Continue(e),
            },
            FINISH => match parse::<FinishArgs>(args) {
                Ok(a) => match self
                    .client
                    .set_review_summary(self.task_id, &a.summary)
                    .await
                {
                    Ok(()) => ToolOutcome::Finish,
                    Err(e) => ToolOutcome::Continue(format!(
                        "error: could not record the summary: {e:#}. Call `finish` again."
                    )),
                },
                Err(e) => ToolOutcome::Continue(format!(
                    "{e} Expected JSON like {{\"summary\": \"…your overall verdict…\"}}."
                )),
            },
            REPORT_PROGRESS => match parse::<NoteArgs>(args) {
                Ok(a) => {
                    tracing::info!(note = %a.note, "review agent progress");
                    ToolOutcome::Continue("acknowledged".to_string())
                }
                Err(e) => ToolOutcome::Continue(e),
            },
            ABORT => match parse::<AbortArgs>(args) {
                Ok(a) => ToolOutcome::Abort(a.reason),
                Err(e) => ToolOutcome::Continue(e),
            },
            other => ToolOutcome::Continue(format!(
                "error: unknown tool {other:?}. Available tools: {VECTOR_SEMANTIC_SEARCH}, \
                 {GRAPH_FIND_SYMBOL}, {GRAPH_GET_CALLERS}, {ADD_REVIEW_COMMENT}, {ADD_COMMENT}, \
                 {FINISH}, {REPORT_PROGRESS}, {ABORT}."
            )),
        }
    }

    async fn vector_search(&self, query: &str, limit: i64) -> ToolOutcome {
        let result = async {
            let mut vectors = self.embedder.embed(&[query]).await?;
            let embedding = vectors
                .pop()
                .ok_or_else(|| anyhow::anyhow!("embeddings API returned no vector"))?;
            let hits = self.client.search(self.task_id, &embedding, limit).await?;
            Ok::<_, anyhow::Error>(hits)
        }
        .await;
        ToolOutcome::Continue(render(VECTOR_SEMANTIC_SEARCH, result))
    }
}

/// Parse a tool call's JSON-string arguments, mapping a failure to a model-facing error string.
fn parse<T: serde::de::DeserializeOwned>(arguments: &str) -> Result<T, String> {
    serde_json::from_str::<T>(arguments).map_err(|e| {
        format!(
            "error: invalid arguments — {e}. Re-call with arguments matching the tool's schema."
        )
    })
}

/// Render a retrieval result as a JSON string for the model, or a recoverable error string.
fn render<T: serde::Serialize>(tool: &str, result: anyhow::Result<T>) -> String {
    match result.and_then(|v| Ok(serde_json::to_string_pretty(&v)?)) {
        Ok(json) => json,
        Err(error) => format!("error: {tool} failed: {error:#}"),
    }
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::native::chat::FunctionCall;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "c1".to_string(),
            kind: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
        }
    }

    fn tools<'a>(cp: &'a ControlPlaneClient, emb: &'a EmbeddingsClient) -> Tools<'a> {
        Tools {
            client: cp,
            embedder: emb,
            task_id: Uuid::nil(),
        }
    }

    #[test]
    fn tool_defs_expose_the_eight_tools_in_order() {
        let names: Vec<_> = tool_defs()
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        assert_eq!(
            names,
            vec![
                VECTOR_SEMANTIC_SEARCH,
                GRAPH_FIND_SYMBOL,
                GRAPH_GET_CALLERS,
                ADD_REVIEW_COMMENT,
                ADD_COMMENT,
                FINISH,
                REPORT_PROGRESS,
                ABORT,
            ]
        );
    }

    // ── Positive: a search call embeds the query, hits the control plane, returns the hits ──────
    #[tokio::test]
    async fn dispatch_vector_search_returns_hits() {
        let emb_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "index": 0, "embedding": [0.1_f32, 0.2_f32] }]
            })))
            .mount(&emb_server)
            .await;
        let cp_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/internal/tasks/{}/search", Uuid::nil())))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "file_path": "src/auth/session.rs", "language": "rust", "chunk_type": "function",
                "symbol_name": "validate_session", "start_line": 40, "end_line": 50,
                "content": "fn validate_session() {}", "score": 0.97
            }])))
            .mount(&cp_server)
            .await;

        let cp = ControlPlaneClient::new(cp_server.uri(), "tok");
        let emb = EmbeddingsClient::new(&emb_server.uri(), "key", "model");
        let outcome = tools(&cp, &emb)
            .dispatch(&call(
                VECTOR_SEMANTIC_SEARCH,
                r#"{"query":"session expiry"}"#,
            ))
            .await;
        match outcome {
            ToolOutcome::Continue(s) => {
                assert!(s.contains("src/auth/session.rs"), "got: {s}");
                assert!(s.contains("validate_session"));
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    // ── Positive: add_review_comment buffers the finding via the control plane ──────────────────
    #[tokio::test]
    async fn dispatch_add_review_comment_buffers() {
        let cp_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/internal/tasks/{}/review/inline",
                Uuid::nil()
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&cp_server)
            .await;
        let cp = ControlPlaneClient::new(cp_server.uri(), "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        let args = r#"{"file":"a.rs","line":7,"title":"No expiry","priority":"P0","category":"security","body":"accepts expired tokens","suggestion":"if expired { return Err }"}"#;
        match tools(&cp, &emb)
            .dispatch(&call(ADD_REVIEW_COMMENT, args))
            .await
        {
            ToolOutcome::Continue(s) => {
                assert!(s.contains("recorded finding at a.rs:7"), "got: {s}")
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    // ── Positive: finish records the summary and ends the run ───────────────────────────────────
    #[tokio::test]
    async fn dispatch_finish_sets_summary_and_finishes() {
        let cp_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/internal/tasks/{}/review/summary",
                Uuid::nil()
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&cp_server)
            .await;
        let cp = ControlPlaneClient::new(cp_server.uri(), "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        match tools(&cp, &emb)
            .dispatch(&call(FINISH, r#"{"summary":"All good."}"#))
            .await
        {
            ToolOutcome::Finish => {}
            other => panic!("expected Finish, got {other:?}"),
        }
    }

    // ── Positive: abort surfaces the reason ─────────────────────────────────────────────────────
    #[tokio::test]
    async fn dispatch_abort_returns_reason() {
        let cp = ControlPlaneClient::new("http://unused", "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        match tools(&cp, &emb)
            .dispatch(&call(ABORT, r#"{"reason":"diff unreadable"}"#))
            .await
        {
            ToolOutcome::Abort(r) => assert_eq!(r, "diff unreadable"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    // ── Negative: a malformed add_review_comment payload is a recoverable Continue ──────────────
    #[tokio::test]
    async fn dispatch_add_review_comment_invalid_is_recoverable() {
        let cp = ControlPlaneClient::new("http://unused", "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        // missing required fields (priority/category/title/body)
        match tools(&cp, &emb)
            .dispatch(&call(ADD_REVIEW_COMMENT, r#"{"file":"a.rs","line":7}"#))
            .await
        {
            ToolOutcome::Continue(s) => assert!(s.to_lowercase().contains("expected"), "hint: {s}"),
            other => panic!("expected Continue (recoverable), got {other:?}"),
        }
    }

    // ── Negative: non-JSON arguments come back as a recoverable error ───────────────────────────
    #[tokio::test]
    async fn dispatch_malformed_arguments_is_recoverable() {
        let cp = ControlPlaneClient::new("http://unused", "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        match tools(&cp, &emb)
            .dispatch(&call(VECTOR_SEMANTIC_SEARCH, "not json"))
            .await
        {
            ToolOutcome::Continue(s) => assert!(s.starts_with("error: invalid arguments")),
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    // ── Negative: an unknown tool name is reported, not fatal ───────────────────────────────────
    #[tokio::test]
    async fn dispatch_unknown_tool_is_reported() {
        let cp = ControlPlaneClient::new("http://unused", "tok");
        let emb = EmbeddingsClient::new("http://unused", "key", "model");
        match tools(&cp, &emb).dispatch(&call("delete_repo", "{}")).await {
            ToolOutcome::Continue(s) => assert!(s.contains("unknown tool")),
            other => panic!("expected Continue, got {other:?}"),
        }
    }
}
