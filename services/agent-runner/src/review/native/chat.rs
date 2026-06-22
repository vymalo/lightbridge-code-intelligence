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

use anyhow::Context;
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

/// The assistant's reply for one turn: its message (text and/or `tool_calls`) plus the provider's
/// `finish_reason` (e.g. `tool_calls`, `stop`, `length`) so the loop can detect truncation.
#[derive(Debug, Clone)]
pub struct Completion {
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
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
    /// is the chat model id (`LLM_MODEL`).
    pub fn new(base_url: &str, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key: api_key.into(),
            model: model.into(),
            http: build_http_client(),
            attribution: reqwest::header::HeaderMap::new(),
        }
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
    pub async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
        params: ChatParams,
    ) -> anyhow::Result<Completion> {
        let request = ChatRequest {
            model: &self.model,
            messages,
            tools,
            tool_choice: (!tools.is_empty()).then_some("auto"),
            temperature: params.temperature,
            top_p: params.top_p,
            max_tokens: params.max_tokens,
        };

        let response: ChatResponse = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .headers(self.attribution.clone())
            .json(&request)
            .send()
            .await
            .context("chat completions request failed")?
            .error_for_status()
            .context("chat completions API returned error")?
            .json()
            .await
            .context("parsing chat completions response")?;

        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("chat completions response had no choices"))?;

        Ok(Completion {
            finish_reason: choice.finish_reason,
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

/// Build the HTTP client, additionally trusting the internal CA PEM at `LLM_CA_CERT` (falling back to
/// `EMBEDDINGS_CA_CERT`, since the chat and embeddings endpoints share the eaig gateway and its
/// private CA — `ClusterIssuer/self-signed-ca`, which the default rustls/webpki roots don't include).
/// Absent both, the default client (public roots) is used. `add_root_certificate` augments the default
/// roots, it doesn't replace them.
fn build_http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
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
        assert!(format!("{err:#}").contains("returned error"));
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
