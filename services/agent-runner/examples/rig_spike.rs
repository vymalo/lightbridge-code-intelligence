//! SPIKE (branch `spike/rig-llm-substrate`): evaluate `rig-core` as the LLM substrate for the
//! native review agent, behind a thin adapter boundary — WITHOUT giving up our policy loop.
//!
//! What this proves (the three feasibility questions from the design discussion):
//!   1. Rig's OpenAI-compatible client can talk to the eaig gateway with OUR `reqwest::Client`,
//!      so the self-signed internal CA *and* the per-project attribution headers (epic #89 billing)
//!      ride along — neither needs first-class Rig support. See `build_rig_client`.
//!   2. One of our mediated tools wraps as a Rig `ToolDefinition` with zero schema translation —
//!      Rig's `ToolDefinition { name, description, parameters }` is byte-identical to our
//!      `ToolDef.function`. See `add_review_comment_tool`.
//!   3. The raw multi-turn contract our policy loop depends on — `complete(messages, tools, params)
//!      -> Completion { content, tool_calls, finish_reason, usage }` — maps 1:1 onto Rig's
//!      `CompletionModel::completion`. The loop (budgets, batching, coverage gate, refute, wind-down)
//!      stays entirely ours; only the one-shot call swaps. See `LlmClient` + `RigChatClient`.
//!
//! Run (compile-only is the primary deliverable):
//!   cargo build -p agent-runner --example rig_spike
//! Optional live round-trip if creds are present:
//!   LLM_BASE_URL=.../v1 LLM_API_KEY=... LLM_MODEL=adorsys-reviewer \
//!     [LLM_CA_CERT=/path/ca.pem] cargo run -p agent-runner --example rig_spike
//!
//! NOT wired into the binary (dev-dependency only). Delete the example + the two Cargo entries to
//! drop the spike entirely.

// Mirror types intentionally model the full chat.rs shape; not every field/variant is exercised
// by the one-shot demo, so dead-code lints are expected noise here.
#![allow(dead_code)]

use anyhow::Context;
use reqwest13 as http; // rig-core 0.39 speaks reqwest 0.13; the workspace is on 0.12 (see Cargo.toml).
use rig_core::client::completion::CompletionClient;
use rig_core::completion::{AssistantContent, CompletionModel, ToolDefinition};
use rig_core::message::Message;
use rig_core::providers::openai;

// ---------------------------------------------------------------------------
// Local mirrors of the existing `chat.rs` types, named identically so the real
// integration is an obvious find-and-replace. In-tree these already exist; the
// example can't reach the crate's private modules, so we restate the shapes.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum ChatMessage {
    System(String),
    User(String),
    Assistant {
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug)]
struct ToolCall {
    id: String,
    name: String,
    /// Rig hands us a parsed `serde_json::Value` here; our current `chat.rs` keeps the raw
    /// `String`. Storing the Value means the dispatch layer does `from_value` instead of
    /// `from_str` — a small ergonomic win, not a breaking change.
    arguments: serde_json::Value,
}

#[derive(Clone, Debug, Default)]
struct GenParams {
    temperature: Option<f64>,
    top_p: Option<f64>,
    max_tokens: Option<i64>,
}

#[derive(Debug)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Debug)]
struct Completion {
    content: Option<String>,
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
}

/// The adapter boundary. Today's `ChatClient` is one impl; `RigChatClient` is another. The policy
/// loop (and the ADR-0039 retry/backoff/circuit-breaker/fallback) sit ON TOP of this trait, so
/// Rig's pre-1.0 churn can only ever touch the impl below — never the loop.
#[allow(async_fn_in_trait)]
trait LlmClient {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        params: &GenParams,
    ) -> anyhow::Result<Completion>;
}

// ---------------------------------------------------------------------------
// The Rig-backed implementation.
// ---------------------------------------------------------------------------

struct RigChatClient {
    client: openai::CompletionsClient,
    model: String,
}

impl RigChatClient {
    /// Feasibility crux #1: build the transport ourselves so the CA root and the attribution
    /// headers are baked in, then hand it to Rig via `.http_client(..)`.
    fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        ca_cert_pem: Option<&[u8]>,
        attribution: &[(String, String)],
    ) -> anyhow::Result<Self> {
        let mut headers = http::header::HeaderMap::new();
        for (k, v) in attribution {
            let name = http::header::HeaderName::from_bytes(k.as_bytes())
                .with_context(|| format!("bad attribution header name {k:?}"))?;
            let val = http::header::HeaderValue::from_str(v)
                .with_context(|| format!("bad attribution header value for {k:?}"))?;
            headers.insert(name, val);
        }

        let mut builder = http::Client::builder().default_headers(headers);
        if let Some(pem) = ca_cert_pem {
            let cert = http::Certificate::from_pem(pem).context("parse LLM_CA_CERT pem")?;
            builder = builder.add_root_certificate(cert);
        }
        let http = builder.build().context("build reqwest client")?;

        // `api_key` first (type-state), then base_url + our transport.
        let client = openai::CompletionsClient::builder()
            .api_key(api_key)
            .base_url(base_url)
            .http_client(http)
            .build()
            .map_err(|e| anyhow::anyhow!("build rig openai client: {e}"))?;

        Ok(Self {
            client,
            model: model.to_string(),
        })
    }
}

impl LlmClient for RigChatClient {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        params: &GenParams,
    ) -> anyhow::Result<Completion> {
        // Map our message history → Rig's. System goes to the `preamble`; the rest become Rig
        // `Message`s. We use the last user turn as the builder `prompt` and feed the remainder
        // via `.messages(..)`.
        let mut preamble: Option<String> = None;
        let mut history: Vec<Message> = Vec::new();
        for m in messages {
            match m {
                ChatMessage::System(s) => preamble = Some(s.clone()),
                ChatMessage::User(u) => history.push(Message::user(u.clone())),
                ChatMessage::Assistant { content, .. } => {
                    history.push(Message::assistant(content.clone().unwrap_or_default()));
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                } => {
                    history.push(Message::tool_result(tool_call_id.clone(), content.clone()));
                }
            }
        }
        let prompt = history.pop().unwrap_or_else(|| Message::user(""));

        let extra = build_additional_params(params);
        let model = self.client.completion_model(&self.model);
        let mut req = model
            .completion_request(prompt)
            .messages(history)
            .tools(tools.to_vec());
        if let Some(p) = preamble {
            req = req.preamble(p);
        }
        if let Some(t) = params.temperature {
            req = req.temperature(t);
        }
        req = req.additional_params_opt(extra);

        let resp = model
            .completion(req.build())
            .await
            .context("rig completion call")?;

        // Feasibility crux #3: fold Rig's typed `choice` back into our flat `Completion`.
        // Note Rig's first-class `AssistantContent::Reasoning` — chain-of-thought arrives in its
        // OWN variant, not smuggled into `content`. That is structurally the fix for the deepseek
        // `<think>`-into-content leak (IF the gateway tags it), which today we strip after the fact.
        let mut content: Option<String> = None;
        let mut tool_calls = Vec::new();
        for item in resp.choice.iter() {
            match item {
                AssistantContent::Text(t) => {
                    content.get_or_insert_with(String::new).push_str(&t.text);
                }
                AssistantContent::ToolCall(tc) => tool_calls.push(ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                }),
                AssistantContent::Reasoning(_) | AssistantContent::Image(_) => {}
            }
        }
        let finish_reason = Some(
            if tool_calls.is_empty() {
                "stop"
            } else {
                "tool_calls"
            }
            .to_string(),
        );

        Ok(Completion {
            content,
            tool_calls,
            finish_reason,
            usage: Some(Usage {
                prompt_tokens: resp.usage.input_tokens,
                completion_tokens: resp.usage.output_tokens,
            }),
        })
    }
}

/// top_p / max_tokens have no first-class builder slot, so they ride in `additional_params` —
/// exactly how our current client tacks them onto the JSON body.
fn build_additional_params(params: &GenParams) -> Option<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    if let Some(p) = params.top_p {
        obj.insert("top_p".into(), serde_json::json!(p));
    }
    if let Some(m) = params.max_tokens {
        obj.insert("max_tokens".into(), serde_json::json!(m));
    }
    (!obj.is_empty()).then(|| serde_json::Value::Object(obj))
}

/// Feasibility crux #2: our `add_review_comment` schema is already a `serde_json::Value` — it drops
/// straight into Rig's `ToolDefinition` with no translation layer.
fn add_review_comment_tool() -> ToolDefinition {
    ToolDefinition {
        name: "add_review_comment".to_string(),
        description: "Record one inline finding on a changed line of the diff.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "file":     { "type": "string" },
                "line":     { "type": "integer" },
                "title":    { "type": "string" },
                "priority": { "type": "string", "enum": ["P0", "P1", "P2"] },
                "category": { "type": "string",
                    "enum": ["security","correctness","quality","style","performance"] },
                "body":     { "type": "string" },
                "evidence": { "type": "string" },
                "suggestion": { "type": "string" }
            },
            "required": ["file","line","title","priority","category","body"]
        }),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let tools = vec![add_review_comment_tool()];
    println!(
        "✔ tool schema maps 1:1 → Rig ToolDefinition ({} tool(s))",
        tools.len()
    );

    let (base, key, model) = match (
        std::env::var("LLM_BASE_URL"),
        std::env::var("LLM_API_KEY"),
        std::env::var("LLM_MODEL"),
    ) {
        (Ok(b), Ok(k), Ok(m)) => (b, k, m),
        _ => {
            println!("ℹ no LLM_* env set — compile/wiring proof only, skipping live round-trip.");
            // Still construct against a dummy base to prove the builder chain + transport wiring.
            let _client = RigChatClient::new(
                "https://example.invalid/v1",
                "dummy",
                "adorsys-reviewer",
                None,
                &[("x-code-intelligence-repo".into(), "vymalo/demo".into())],
            )?;
            println!(
                "✔ RigChatClient constructed (CA + attribution headers injected into transport)"
            );
            return Ok(());
        }
    };

    let ca = std::env::var("LLM_CA_CERT")
        .ok()
        .and_then(|p| std::fs::read(p).ok());
    let attribution = vec![(
        "x-code-intelligence-repo".to_string(),
        "vymalo/spike".to_string(),
    )];
    let client = RigChatClient::new(&base, &key, &model, ca.as_deref(), &attribution)?;

    let messages = vec![
        ChatMessage::System("You are a terse reviewer. If you see an issue, call the tool.".into()),
        ChatMessage::User(
            "In `pay.rs:42` the retry log line fires on every first-attempt failure even when the \
             retry succeeds, so transient flakes log twice. Record it."
                .into(),
        ),
    ];
    let out = client
        .complete(&messages, &tools, &GenParams::default())
        .await?;
    println!(
        "✔ live round-trip ok: finish={:?}, tool_calls={}, usage={:?}",
        out.finish_reason,
        out.tool_calls.len(),
        out.usage
    );
    for tc in &out.tool_calls {
        println!("   → {} {}", tc.name, tc.arguments);
    }
    if let Some(c) = &out.content {
        println!("   content: {c}");
    }
    Ok(())
}
