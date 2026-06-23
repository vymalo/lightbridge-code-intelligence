//! OpenAI-compatible **Chat Completions** client with function/tool calling (ADR-0026).
//!
//! Talks to `POST {base}/chat/completions` on the eaig gateway — the same gateway and key the review
//! model already uses (`LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL`, see
//! [`ReviewConfig`](crate::bootstrap::config::ReviewConfig)). Unlike the embeddings base URL, the LLM
//! base URL already includes the `/v1` segment, so we only append `/chat/completions`.
//!
//! This is the transport layer for the native agent loop: it serializes the multi-turn `messages`
//! array (system / user / assistant-with-tool-calls / tool-result), advertises the available `tools`,
//! and returns the assistant's reply — which is either text or a set of `tool_calls` the loop then
//! dispatches. It deliberately knows nothing about *which* tools exist or *how* to run them; that is
//! the dispatcher's job (a later PR).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A single message in the Chat Completions `messages` array.
///
/// The same type is used both for messages we send (system/user prompts, tool results, and the
/// assistant turns we echo back) and for the assistant reply we parse out of a response — hence the
/// optional fields: a `tool` message carries `tool_call_id` + `content`; an assistant turn that calls
/// tools carries `tool_calls` and often no `content`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Set only on `role = "tool"` messages — ties a tool result back to the call it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// A `system` message — the reviewer guidance + output contract.
    pub fn system(content: impl Into<String>) -> Self {
        Self::text("system", content)
    }

    /// A `user` message — the requested command + the diff/context.
    pub fn user(content: impl Into<String>) -> Self {
        Self::text("user", content)
    }

    /// A `tool` message carrying the result of a tool call back to the model.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

/// A tool call the model wants the loop to execute. `arguments` is a JSON-encoded **string** (per the
/// OpenAI spec), not an object — the dispatcher parses it against the tool's parameter schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded arguments (a string, e.g. `"{\"query\":\"auth\"}"`).
    #[serde(default)]
    pub arguments: String,
}

fn function_kind() -> String {
    "function".to_string()
}

/// `skip_serializing_if` helper for a borrowed slice field (the predicate receives `&&[T]`).
fn slice_is_empty<T>(slice: &&[T]) -> bool {
    slice.is_empty()
}

/// A tool advertised to the model in the request's `tools` array.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    /// JSON Schema for the function's parameters.
    pub parameters: serde_json::Value,
}

impl ToolDef {
    /// Build a `function`-type tool definition.
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            kind: "function".to_string(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// Per-request generation parameters. All optional — `None` leaves the provider/model default. Mirrors
/// the knobs on [`ReviewConfig`](crate::bootstrap::config::ReviewConfig) (#71).
#[derive(Debug, Clone, Copy, Default)]
pub struct ChatParams {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
}

/// The assistant's reply for one turn: its message (text and/or `tool_calls`), the provider's
/// `finish_reason` (e.g. `tool_calls`, `stop`, `length`) so the loop can detect truncation, and the
/// token `usage` for the turn (for the transcript/observability, ADR-0034).
#[derive(Debug, Clone)]
pub struct Completion {
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// Token usage for one completion, as reported by the OpenAI-compatible API. All optional — some
/// gateways omit it.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "slice_is_empty")]
    tools: &'a [ToolDef],
    /// Only sent alongside `tools`; `"auto"` lets the model choose when to call them.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

/// Retry/backoff policy for one chat turn (ADR-0039). Retries fire **only** on transient failures
/// (connect/timeout, HTTP 429, HTTP 5xx); a 4xx other than 429 is deterministic and never retried.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Retries on a transient failure (total attempts = `max_retries + 1`).
    pub max_retries: u32,
    /// Base backoff; attempt *n* (0-indexed) sleeps roughly `base * 2^n` plus deterministic jitter.
    pub base_backoff: Duration,
    /// Ceiling on a single backoff so a high attempt count can't sleep absurdly long.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    /// Backoff for `attempt` (0 = the wait *before* the first retry). Exponential, capped at
    /// `max_backoff`, plus a small **deterministic** jitter seeded by the attempt index — so the
    /// schedule is reproducible in tests (no clock, no RNG) yet de-synchronises retries a little.
    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 1u64 << attempt.min(16); // 2^attempt, clamped so the shift can't overflow
        let base = self
            .base_backoff
            .saturating_mul(factor.min(u32::MAX as u64) as u32);
        let capped = base.min(self.max_backoff);
        // Deterministic jitter in [0, 250ms): a cheap hash of the attempt index, no SystemTime/RNG.
        let jitter_ms = (attempt as u64).wrapping_mul(2_654_435_761) % 250;
        capped.saturating_add(Duration::from_millis(jitter_ms))
    }
}

/// Why a turn failed, so the loop can decide whether a transient error is worth a retry/failover vs.
/// a deterministic one that should fail fast.
#[derive(Debug)]
pub struct ChatError {
    pub error: anyhow::Error,
    /// `true` for connect/timeout, HTTP 429, or HTTP 5xx — the loop retries/fails over on these only.
    pub transient: bool,
    /// `Retry-After` seconds parsed off a 429, when present — the loop honours it over its own backoff.
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.error)
    }
}

/// Chat Completions client for the review model.
pub struct ChatClient {
    url: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
    /// Gateway attribution headers (epic #89), added to every request so token spend is billed to the
    /// right project. Empty unless set via [`ChatClient::with_attribution`].
    attribution: reqwest::header::HeaderMap,
}

impl ChatClient {
    /// `base_url` is the LLM gateway base **including** the `/v1` segment (`LLM_BASE_URL`); the model
    /// is the chat model id (`LLM_MODEL`). Uses the default per-request timeout
    /// ([`DEFAULT_REQUEST_TIMEOUT_SECS`](crate::bootstrap::config::DEFAULT_REQUEST_TIMEOUT_SECS)).
    pub fn new(base_url: &str, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_timeout(
            base_url,
            api_key,
            model,
            Duration::from_secs(crate::bootstrap::config::DEFAULT_REQUEST_TIMEOUT_SECS),
        )
    }

    /// Like [`new`](Self::new) but with an explicit per-request timeout (ADR-0039). eaig can take
    /// ~2 minutes per turn, so callers pass a generous value (default 180s).
    pub fn with_timeout(
        base_url: &str,
        api_key: impl Into<String>,
        model: impl Into<String>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: api_key.into(),
            model: model.into(),
            http: build_http_client(request_timeout),
            attribution: reqwest::header::HeaderMap::new(),
        }
    }

    /// Return a copy of this client that targets a different model id (same gateway/key/timeout) — used
    /// for failover to the secondary model (ADR-0039). Cheap: clones the shared `reqwest::Client`.
    pub fn for_model(&self, model: impl Into<String>) -> Self {
        Self {
            url: self.url.clone(),
            api_key: self.api_key.clone(),
            model: model.into(),
            http: self.http.clone(),
            attribution: self.attribution.clone(),
        }
    }

    /// The model id this client targets.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Attach gateway attribution headers (epic #89). Unparseable header names/values are skipped (the
    /// keys are our own controlled values, so that shouldn't happen).
    pub fn with_attribution(mut self, headers: &[(String, String)]) -> Self {
        use reqwest::header::{HeaderName, HeaderValue};
        for (name, value) in headers {
            match (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                (Ok(n), Ok(v)) => {
                    self.attribution.insert(n, v);
                }
                _ => tracing::warn!(header = %name, "skipping unparseable attribution header"),
            }
        }
        self
    }

    /// One completion turn: send the conversation so far + the advertised `tools`, return the
    /// assistant's reply. `tools` may be empty for a plain completion.
    ///
    /// On a non-2xx response the **response body** is read (bounded) and folded into the error, so a
    /// gateway rejection (bad model, quota, validation) surfaces a real reason instead of a bare
    /// status code — this is the key fix for "the review failed without saying why" (ADR-0039).
    pub async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        params: ChatParams,
    ) -> anyhow::Result<Completion> {
        self.complete_inner(messages, tools, params)
            .await
            .map_err(|e| e.error)
    }

    /// [`complete`](Self::complete) with retry/backoff on transient failures (ADR-0039). Retries up to
    /// `policy.max_retries` times on connect/timeout, HTTP 429, or HTTP 5xx — honouring a 429's
    /// `Retry-After` over the computed backoff — and returns immediately on success or a deterministic
    /// 4xx. The returned [`ChatError`] tells the caller whether the *final* failure was transient (so
    /// the loop can decide on failover).
    pub async fn complete_with_retry(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        params: ChatParams,
        policy: RetryPolicy,
    ) -> Result<Completion, ChatError> {
        let mut attempt = 0u32;
        loop {
            match self.complete_inner(messages, tools, params).await {
                Ok(completion) => return Ok(completion),
                Err(err) => {
                    if !err.transient || attempt >= policy.max_retries {
                        return Err(err);
                    }
                    let wait = err.retry_after.map(|d| d.min(policy.max_backoff)).unwrap_or_else(|| policy.backoff(attempt));
                    tracing::warn!(
                        model = %self.model,
                        attempt = attempt + 1,
                        max_retries = policy.max_retries,
                        backoff_ms = wait.as_millis() as u64,
                        retry_after = err.retry_after.is_some(),
                        error = %err,
                        "transient chat failure; retrying after backoff"
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
            }
        }
    }

    /// The single-attempt request, returning a classified [`ChatError`] on failure.
    async fn complete_inner(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        params: ChatParams,
    ) -> Result<Completion, ChatError> {
        let request = ChatRequest {
            model: &self.model,
            messages,
            tools,
            tool_choice: (!tools.is_empty()).then_some("auto"),
            temperature: params.temperature,
            top_p: params.top_p,
            max_tokens: params.max_tokens,
        };

        let response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .headers(self.attribution.clone())
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                // A transport error (connect refused, DNS, or our own per-request timeout) is transient.
                let transient = e.is_timeout() || e.is_connect() || e.is_request();
                ChatError {
                    error: anyhow::Error::new(e).context("chat completions request failed"),
                    transient,
                    retry_after: None,
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            // Read the body (bounded) so the failure is legible. 429 + 5xx are transient; other 4xx
            // are deterministic (bad request, auth, unknown model) and must NOT be retried.
            let retry_after = retry_after_secs(response.headers());
            let body = response.text().await.unwrap_or_default();
            let snippet = truncate_on_boundary(&body, 1024);
            let transient =
                status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            return Err(ChatError {
                error: anyhow::anyhow!(
                    "chat completions API returned {status}: {}",
                    if snippet.is_empty() {
                        "(empty body)"
                    } else {
                        snippet
                    }
                ),
                transient,
                retry_after: retry_after.filter(|_| transient),
            });
        }

        let response: ChatResponse = response.json().await.map_err(|e| ChatError {
            // A malformed 2xx body is not a transport problem — don't retry it.
            error: anyhow::Error::new(e).context("parsing chat completions response"),
            transient: false,
            retry_after: None,
        })?;

        let usage = response.usage;
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ChatError {
                error: anyhow::anyhow!("chat completions response had no choices"),
                transient: false,
                retry_after: None,
            })?;

        Ok(Completion {
            finish_reason: choice.finish_reason,
            usage,
            message: ChatMessage {
                role: choice
                    .message
                    .role
                    .unwrap_or_else(|| "assistant".to_string()),
                content: choice.message.content,
                tool_calls: choice.message.tool_calls,
                tool_call_id: None,
            },
        })
    }
}

/// `s` truncated to at most `max` bytes, never slicing through a multi-byte char (mirrors the helper
/// in the agent loop; kept local so the transport layer has no cross-module dep).
fn truncate_on_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Parse a `Retry-After` header expressed as an integer number of seconds (the form gateways use for
/// 429s). The HTTP-date form is ignored — we fall back to our own backoff then.
fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Build the HTTP client, additionally trusting the internal CA PEM at `LLM_CA_CERT` (falling back to
/// `EMBEDDINGS_CA_CERT`, since the chat and embeddings endpoints share the eaig gateway and its
/// private CA — `ClusterIssuer/self-signed-ca`, which the default rustls/webpki roots don't include).
/// Absent both, the default client (public roots) is used. `add_root_certificate` augments the default
/// roots, it doesn't replace them.
fn build_http_client(request_timeout: Duration) -> reqwest::Client {
    // Per-request timeout (ADR-0039): generous (default 180s) because eaig can legitimately take up to
    // ~2 minutes per turn — an aggressive timeout would kill a slow-but-valid response.
    let mut builder = reqwest::Client::builder().timeout(request_timeout);
    if let Some((path, cert)) = ca_cert() {
        match cert {
            Ok(cert) => {
                builder = builder.add_root_certificate(cert);
                tracing::info!(%path, "chat: trusting extra CA");
            }
            Err(error) => {
                tracing::warn!(%error, %path, "chat: could not load CA cert; using default roots");
            }
        }
    }
    builder
        .build()
        .expect("building the chat HTTP client with default roots cannot fail")
}

/// The first configured internal-CA path (`LLM_CA_CERT`, else `EMBEDDINGS_CA_CERT`) and its parsed
/// certificate, or `None` when neither is set.
fn ca_cert() -> Option<(String, anyhow::Result<reqwest::Certificate>)> {
    for var in ["LLM_CA_CERT", "EMBEDDINGS_CA_CERT"] {
        if let Ok(path) = std::env::var(var) {
            let cert = std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|pem| Ok(reqwest::Certificate::from_pem(&pem)?));
            return Some((path, cert));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn search_tool() -> ToolDef {
        ToolDef::function(
            "vector_semantic_search",
            "Search the repo by meaning.",
            serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
            }),
        )
    }

    #[tokio::test]
    async fn complete_sends_model_messages_tools_and_parses_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(bearer_token("key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "index": 0,
                    "finish_reason": "tool_calls",
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "vector_semantic_search",
                                "arguments": "{\"query\":\"session expiry\"}"
                            }
                        }]
                    }
                }]
            })))
            .mount(&server)
            .await;

        // base URL includes /v1, like LLM_BASE_URL.
        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "qwen-coder");
        let out = client
            .complete(
                &[
                    ChatMessage::system("review the diff"),
                    ChatMessage::user("@lightbridge review"),
                ],
                &[search_tool()],
                ChatParams {
                    temperature: Some(0.2),
                    max_tokens: Some(4096),
                    ..ChatParams::default()
                },
            )
            .await
            .expect("complete");

        assert_eq!(out.finish_reason.as_deref(), Some("tool_calls"));
        assert!(out.message.content.is_none());
        assert_eq!(out.message.tool_calls.len(), 1);
        let call = &out.message.tool_calls[0];
        assert_eq!(call.id, "call_1");
        assert_eq!(call.function.name, "vector_semantic_search");
        assert_eq!(call.function.arguments, "{\"query\":\"session expiry\"}");

        // The request we sent carries the model, both messages, the advertised tool, tool_choice, and
        // the generation params; unset params are omitted.
        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "qwen-coder");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            body["tools"][0]["function"]["name"],
            "vector_semantic_search"
        );
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["temperature"], serde_json::json!(0.2));
        assert_eq!(body["max_tokens"], serde_json::json!(4096));
        assert!(body.get("top_p").is_none(), "unset params are omitted");
    }

    #[tokio::test]
    async fn complete_parses_a_plain_text_reply_and_omits_tools_when_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "finish_reason": "stop",
                    "message": { "role": "assistant", "content": "looks good" }
                }]
            })))
            .mount(&server)
            .await;

        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "m");
        let out = client
            .complete(&[ChatMessage::user("hi")], &[], ChatParams::default())
            .await
            .expect("complete");
        assert_eq!(out.message.content.as_deref(), Some("looks good"));
        assert!(out.message.tool_calls.is_empty());

        // With no tools, neither `tools` nor `tool_choice` is sent.
        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[tokio::test]
    async fn complete_surfaces_an_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "m");
        let err = client
            .complete(&[ChatMessage::user("hi")], &[], ChatParams::default())
            .await
            .expect_err("500 is an error");
        assert!(format!("{err:#}").contains("returned 500"), "got: {err:#}");
    }

    // ── ADR-0039 resilience tests ───────────────────────────────────────────────────────────────

    fn ok_reply() -> serde_json::Value {
        serde_json::json!({
            "choices": [{ "finish_reason": "stop",
                "message": { "role": "assistant", "content": "ok" } }]
        })
    }

    fn fast_policy() -> RetryPolicy {
        // Tiny backoff so tests don't actually sleep meaningfully.
        RetryPolicy {
            max_retries: 2,
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
        }
    }

    // A 5xx is transient → retried; once the gateway recovers, the turn succeeds.
    #[tokio::test]
    async fn retries_on_5xx_then_succeeds() {
        let server = MockServer::start().await;
        // First call: 503. Then: 200.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_reply()))
            .mount(&server)
            .await;

        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "m");
        let out = client
            .complete_with_retry(
                &[ChatMessage::user("hi")],
                &[],
                ChatParams::default(),
                fast_policy(),
            )
            .await
            .expect("recovers after a transient 503");
        assert_eq!(out.message.content.as_deref(), Some("ok"));
    }

    // A 400 is deterministic → NOT retried (exactly one request hits the server) and the body surfaces.
    #[tokio::test]
    async fn does_not_retry_on_400_and_surfaces_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": { "message": "unknown model 'm'" }
            })))
            .mount(&server)
            .await;

        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "m");
        let err = client
            .complete_with_retry(
                &[ChatMessage::user("hi")],
                &[],
                ChatParams::default(),
                fast_policy(),
            )
            .await
            .expect_err("400 is deterministic");
        assert!(!err.transient, "400 is not transient");
        let msg = format!("{err}");
        assert!(msg.contains("returned 400"), "status surfaced: {msg}");
        assert!(msg.contains("unknown model"), "body surfaced: {msg}");

        // Exactly one request — no retry.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1, "400 must not be retried");
    }

    // The HTTP error body is folded into the error returned by the plain `complete`.
    #[tokio::test]
    async fn error_body_is_surfaced_in_complete() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(502).set_body_string("upstream connect error or disconnect"),
            )
            .mount(&server)
            .await;

        let client = ChatClient::new(&format!("{}/v1", server.uri()), "key", "m");
        let err = client
            .complete(&[ChatMessage::user("hi")], &[], ChatParams::default())
            .await
            .expect_err("502");
        let msg = format!("{err:#}");
        assert!(msg.contains("502"), "status: {msg}");
        assert!(msg.contains("upstream connect error"), "body: {msg}");
    }

    // A per-request timeout aborts a slow response and classifies it transient.
    #[tokio::test]
    async fn times_out_a_slow_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(ok_reply())
                    .set_delay(Duration::from_millis(300)),
            )
            .mount(&server)
            .await;

        // 50ms timeout < 300ms delay → times out.
        let client = ChatClient::with_timeout(
            &format!("{}/v1", server.uri()),
            "key",
            "m",
            Duration::from_millis(50),
        );
        let err = client
            .complete_inner(&[ChatMessage::user("hi")], &[], ChatParams::default())
            .await
            .expect_err("should time out");
        assert!(err.transient, "a timeout is transient");
        assert!(format!("{err}").contains("request failed"), "got: {err}");
    }

    // `for_model` retargets the model id while sharing the gateway/key/timeout — the failover primitive.
    #[tokio::test]
    async fn for_model_retargets_the_model_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_reply()))
            .mount(&server)
            .await;

        let primary = ChatClient::new(&format!("{}/v1", server.uri()), "key", "primary-model");
        let secondary = primary.for_model("fallback-model");
        assert_eq!(secondary.model(), "fallback-model");
        secondary
            .complete(&[ChatMessage::user("hi")], &[], ChatParams::default())
            .await
            .expect("fallback client works");

        let reqs = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "fallback-model");
    }

    // Backoff is exponential, capped, and jittered deterministically (no clock/RNG in tests).
    #[test]
    fn backoff_is_exponential_capped_and_deterministic() {
        let p = RetryPolicy {
            max_retries: 5,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        };
        // attempt 0 ≈ 100ms, attempt 1 ≈ 200ms, attempt 2 ≈ 400ms (plus <250ms jitter).
        assert!(p.backoff(0) >= Duration::from_millis(100));
        assert!(p.backoff(0) < Duration::from_millis(350));
        assert!(p.backoff(2) >= Duration::from_millis(400));
        // High attempt is capped near max_backoff (+ jitter), never unbounded.
        assert!(p.backoff(20) <= Duration::from_secs(2) + Duration::from_millis(250));
        // Deterministic: same input → same output.
        assert_eq!(p.backoff(3), p.backoff(3));
    }

    #[test]
    fn tool_messages_serialize_with_id_and_assistant_tool_calls_round_trip() {
        // A tool-result message carries role + content + tool_call_id, no tool_calls.
        let tool_msg = ChatMessage::tool("call_1", "results...");
        let v = serde_json::to_value(&tool_msg).unwrap();
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_1");
        assert!(v.get("tool_calls").is_none(), "empty tool_calls omitted");

        // An assistant turn with tool_calls round-trips (we echo it back into the next request).
        let assistant = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                kind: "function".to_string(),
                function: FunctionCall {
                    name: "submit_findings".to_string(),
                    arguments: "{}".to_string(),
                },
            }],
            tool_call_id: None,
        };
        let round: ChatMessage =
            serde_json::from_value(serde_json::to_value(&assistant).unwrap()).unwrap();
        assert_eq!(round, assistant);
    }
}
