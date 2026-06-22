//! The native review agent loop (ADR-0026).
//!
//! Drives the [`ChatClient`] over the eaig gateway with the [`tool_defs`] surface: system+diff prompt
//! → model → (retrieval tool calls) → … → `submit_findings`. The review is returned by the model
//! **calling `submit_findings`**, validated at the tool boundary — no stdout scraping. The loop is
//! bounded ([`MAX_TURNS`]) and treats a tool/argument error as a recoverable turn (the model gets the
//! error text back and retries), only failing on `abort`, an exhausted turn budget, or a transport
//! error. Cooperative cancellation comes for free: `run()` races the whole task (this loop included)
//! against the self-cancel poll, so a cancelled task drops the in-flight future.

use anyhow::Context;
use uuid::Uuid;

use super::chat::{ChatClient, ChatMessage, ChatParams};
use super::tools::{ask_tool_defs, tool_defs, ToolOutcome, Tools};
use crate::bootstrap::client::ControlPlaneClient;
use crate::bootstrap::config::ReviewConfig;
use crate::clone::PrDiff;
use crate::indexer::embeddings::EmbeddingsClient;
use crate::review::{ReviewResult, DEFAULT_ASK_GUIDANCE, DEFAULT_REVIEW_GUIDANCE};

/// Hard ceiling on model turns, so a model that never submits can't loop forever (each turn is one
/// chat round-trip; tool calls within a turn don't count separately).
const MAX_TURNS: usize = 16;

/// The native output contract — the analogue of the OpenCode `OUTPUT_CONTRACT`, but the review is
/// returned by **calling `submit_findings`** rather than emitting a JSON block. Appended after the
/// (operator-overridable) guidance, so it's always the final instruction.
const NATIVE_CONTRACT: &str = "\
Return your review by calling the `submit_findings` tool exactly once — never as prose or a code \
block. Ground every claim with the search/graph tools before reporting it; you may not edit files or \
run commands.\n\
- Scope rule (non-negotiable): every finding's `line` MUST be a line this diff adds or changes; \
never comment on untouched code.\n\
- Review ALL dimensions — security, correctness, quality, style, performance. Give each finding a \
`category` (one of those) and a `priority`: `P0` (must fix — bugs, security, data loss), `P1` (should \
fix), or `P2` (minor / nit). Reserve P0/P1 for real harm and file style/quality observations as P2, so \
low-value notes never crowd out blockers.\n\
- Each finding: a short `title`, a `body` (why it matters), an optional `suggestion` (the EXACT \
replacement source for that one line — no diff markers or fences), and optional `resources` (URLs).\n\
- If the change is sound, call `submit_findings` with an empty `findings` array and a one-line \
summary — silence is better than noise.\n\
- If you genuinely cannot produce a useful review, call `abort` with a reason.";

/// How a finished agent run ended: the model called one of the terminal tools. `review` and `ask`
/// share the same loop and differ only in which terminal tool they advertise (so only one variant is
/// reachable per run); the public wrappers map this to their own return type.
enum Terminal {
    Findings(ReviewResult),
    Answer(String),
}

/// The one agent loop both run kinds share (ADR-0026): drive the [`ChatClient`] over a tool surface
/// until the model calls a terminal tool, bounded by [`MAX_TURNS`]. The *only* things that vary
/// between a `review` and an `ask` are the advertised tools (`defs`) and the seeded `messages`
/// (persona + contract); everything else — retrieval, recovery from a bad tool call, cancellation,
/// the turn budget — is identical, so it lives here once. A tool/argument error is fed back to the
/// model as a recoverable turn; only `abort`, an exhausted budget, or a transport error fail the run.
/// `label` names the run in errors/log context.
#[allow(clippy::too_many_arguments)]
async fn drive_agent(
    review: &ReviewConfig,
    defs: &[super::chat::ToolDef],
    mut messages: Vec<ChatMessage>,
    attribution: &[(String, String)],
    client: &ControlPlaneClient,
    embedder: &EmbeddingsClient,
    task_id: Uuid,
    label: &str,
) -> anyhow::Result<Terminal> {
    let chat = ChatClient::new(&review.base_url, &review.api_key, &review.model)
        .with_attribution(attribution);
    let tools = Tools {
        client,
        embedder,
        task_id,
    };
    let params = ChatParams {
        temperature: review.temperature,
        top_p: review.top_p,
        max_tokens: review.max_tokens,
    };

    for turn in 0..MAX_TURNS {
        let completion = chat
            .complete(&messages, defs, params)
            .await
            .with_context(|| format!("{label} chat turn {turn}"))?;
        let assistant = completion.message;
        let calls = assistant.tool_calls.clone();
        // Echo the assistant turn (with its tool_calls) back into the conversation, as the protocol
        // requires before the matching tool-result messages.
        messages.push(assistant);

        if calls.is_empty() {
            // The model answered in prose instead of using the tools — steer it back to the contract.
            messages.push(ChatMessage::user(
                "Use the provided tools to investigate, then call the submit tool for this run \
                 (or `abort` with a reason). Do not reply in prose.",
            ));
            continue;
        }

        for call in &calls {
            match tools.dispatch(call).await {
                ToolOutcome::Submit(result) => return Ok(Terminal::Findings(result)),
                ToolOutcome::Answer(answer) => return Ok(Terminal::Answer(answer)),
                ToolOutcome::Abort(reason) => anyhow::bail!("{label} agent aborted: {reason}"),
                ToolOutcome::Continue(result) => {
                    messages.push(ChatMessage::tool(call.id.as_str(), result));
                }
            }
        }
    }

    anyhow::bail!("{label} agent did not finish within {MAX_TURNS} turns")
}

/// Run the native review loop and return the structured result the model submits. A thin adapter over
/// [`drive_agent`] with the review tool surface ([`tool_defs`]); the only terminal it expects is
/// `submit_findings` (the `ask`-only [`Terminal::Answer`] is unreachable since `submit_answer` isn't
/// advertised here, but is mapped to an error defensively).
#[allow(clippy::too_many_arguments)]
pub async fn run_native_review(
    review: &ReviewConfig,
    command: &str,
    diff: Option<&PrDiff>,
    attribution: &[(String, String)],
    client: &ControlPlaneClient,
    embedder: &EmbeddingsClient,
    task_id: Uuid,
) -> anyhow::Result<ReviewResult> {
    let messages = build_messages(review, command, diff);
    match drive_agent(
        review,
        &tool_defs(),
        messages,
        attribution,
        client,
        embedder,
        task_id,
        "review",
    )
    .await?
    {
        Terminal::Findings(result) => Ok(result),
        Terminal::Answer(_) => anyhow::bail!("review run produced an answer instead of findings"),
    }
}

/// The native **ask** contract (ADR-0033): the analogue of [`NATIVE_CONTRACT`] for a conversational
/// answer. The run ends when the model calls `submit_answer`. Appended after the ask guidance.
const ASK_CONTRACT: &str = "\
Answer the user's question about this pull request and the codebase. Investigate with the \
search/graph tools before answering — ground every claim in what you find; you may not edit files \
or run commands.\n\
- Lead with the answer, then the why. Be direct and concrete; GitHub-flavored Markdown, short code \
blocks where they help.\n\
- When you propose a change, show the concrete code — you are advising, not committing it.\n\
- When you have the answer, call `submit_answer` exactly once with your full Markdown reply. Do not \
answer in prose outside the tool.\n\
- If you genuinely cannot answer, call `abort` with a reason.";

/// Run the native **ask** loop (ADR-0033) and return the Markdown answer the model submits. The
/// mirror of [`run_native_review`] over the `ask` tool surface ([`ask_tool_defs`]) — same loop, the
/// run just ends with `submit_answer` instead of `submit_findings`.
#[allow(clippy::too_many_arguments)]
pub async fn run_native_ask(
    review: &ReviewConfig,
    question: &str,
    diff: Option<&PrDiff>,
    attribution: &[(String, String)],
    client: &ControlPlaneClient,
    embedder: &EmbeddingsClient,
    task_id: Uuid,
) -> anyhow::Result<String> {
    let messages = build_ask_messages(review, question, diff);
    match drive_agent(
        review,
        &ask_tool_defs(),
        messages,
        attribution,
        client,
        embedder,
        task_id,
        "ask",
    )
    .await?
    {
        Terminal::Answer(answer) => Ok(answer),
        Terminal::Findings(_) => anyhow::bail!("ask run produced findings instead of an answer"),
    }
}

/// Assemble the system (ask guidance + contract) and user (question + diff as context) messages for
/// an ask run. The diff, when present, is supplied as context to ground the answer — there is no
/// scope rule (an answer isn't pinned to diff lines).
fn build_ask_messages(
    review: &ReviewConfig,
    question: &str,
    diff: Option<&PrDiff>,
) -> Vec<ChatMessage> {
    let system = format!("{DEFAULT_ASK_GUIDANCE}\n\n{ASK_CONTRACT}");

    let mut user = format!("The user asked: {question}");
    if let Some(pr) = diff {
        user.push_str(&format!(
            "\n\nFor context, this pull request changes {} file(s):\n{}",
            pr.files.len(),
            pr.files
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
        user.push_str("\n\nUnified diff (context for your answer):\n```diff\n");
        user.push_str(truncate_on_boundary(&pr.diff, review.max_diff_chars));
        if pr.diff.len() > review.max_diff_chars {
            user.push_str("\n… [diff truncated] …");
        }
        user.push_str("\n```");
    }

    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// Assemble the system (guidance + contract) and user (command + diff) messages. Mirrors the OpenCode
/// prompt assembly, minus the JSON-block contract (the native agent submits via a tool).
fn build_messages(review: &ReviewConfig, command: &str, diff: Option<&PrDiff>) -> Vec<ChatMessage> {
    let guidance = review
        .system_prompt
        .as_deref()
        .unwrap_or(DEFAULT_REVIEW_GUIDANCE);
    let system = format!("{guidance}\n\n{NATIVE_CONTRACT}");

    let mut user = format!("Requested review command: {command}");
    match diff {
        Some(pr) => {
            user.push_str(&format!(
                "\n\nThis PR changes {} file(s):\n{}",
                pr.files.len(),
                pr.files
                    .iter()
                    .map(|f| format!("- {f}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
            user.push_str("\n\nUnified diff (review ONLY lines this diff changes):\n```diff\n");
            user.push_str(truncate_on_boundary(&pr.diff, review.max_diff_chars));
            if pr.diff.len() > review.max_diff_chars {
                user.push_str("\n… [diff truncated; review the hunks shown above] …");
            }
            user.push_str("\n```");
        }
        None => user.push_str(
            "\n\nNo diff is available for this run; review the working tree for the requested \
             change and keep findings grounded in the tools.",
        ),
    }

    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// `s` truncated to at most `max` bytes, never slicing through a multi-byte char.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::config::ReviewAgent;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// A scripted chat endpoint: returns `responses[i]` on the i-th call, repeating the last forever
    /// (so a loop that never submits keeps getting the same reply).
    struct Script {
        calls: Arc<AtomicUsize>,
        responses: Vec<serde_json::Value>,
    }
    impl Respond for Script {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            let body = self
                .responses
                .get(i)
                .or_else(|| self.responses.last())
                .cloned()
                .unwrap();
            ResponseTemplate::new(200).set_body_json(body)
        }
    }

    fn tool_call_reply(name: &str, arguments: &str) -> serde_json::Value {
        json!({ "choices": [{ "finish_reason": "tool_calls", "message": {
            "role": "assistant",
            "tool_calls": [{ "id": "c1", "type": "function",
                "function": { "name": name, "arguments": arguments } }]
        }}]})
    }

    fn text_reply(text: &str) -> serde_json::Value {
        json!({ "choices": [{ "finish_reason": "stop",
            "message": { "role": "assistant", "content": text } }]})
    }

    async fn mount_chat(server: &MockServer, responses: Vec<serde_json::Value>) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(Script {
                calls: Arc::new(AtomicUsize::new(0)),
                responses,
            })
            .mount(server)
            .await;
    }

    fn review_config(chat_base_with_v1: String) -> ReviewConfig {
        ReviewConfig {
            agent: ReviewAgent::Native,
            base_url: chat_base_with_v1,
            api_key: "k".to_string(),
            model: "m".to_string(),
            system_prompt: None,
            max_diff_chars: 60_000,
            temperature: None,
            top_p: None,
            max_tokens: None,
        }
    }

    // The user's free-text instruction (carried from the @mention comment, #138) reaches the prompt.
    #[test]
    fn build_messages_carries_the_instruction_into_the_user_prompt() {
        let review = review_config("http://unused/v1".to_string());
        let msgs = build_messages(&review, "propose a better implementation", None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        let user = msgs[1].content.as_deref().expect("user content");
        assert!(
            user.contains("propose a better implementation"),
            "instruction must reach the prompt; got: {user}"
        );
    }

    // ── Positive e2e: search → results → submit_findings → validated ReviewResult ───────────────
    #[tokio::test]
    async fn native_loop_searches_then_submits() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![
                tool_call_reply(
                    "lightbridge_vector_semantic_search",
                    r#"{"query":"session expiry"}"#,
                ),
                tool_call_reply(
                    "submit_findings",
                    r#"{"summary":"Missing expiry check.","findings":[
                        {"file":"a.rs","line":7,"priority":"P0","category":"security","title":"No expiry","body":"accepts expired tokens"}
                    ]}"#,
                ),
            ],
        )
        .await;

        // The search tool needs the embeddings + control-plane search endpoints.
        let emb = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "index": 0, "embedding": [0.1_f32, 0.2_f32] }]
            })))
            .mount(&emb)
            .await;
        let cp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/internal/tasks/{}/search", Uuid::nil())))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "file_path": "a.rs", "language": "rust", "chunk_type": "function",
                "symbol_name": "validate", "start_line": 1, "end_line": 9,
                "content": "fn validate() {}", "score": 0.9
            }])))
            .mount(&cp)
            .await;

        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new(cp.uri(), "tok");
        let embc = EmbeddingsClient::new(&emb.uri(), "key", "model");
        let result = run_native_review(
            &review,
            "@lightbridge review",
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
        )
        .await
        .expect("review");
        assert_eq!(result.summary, "Missing expiry check.");
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].file, "a.rs");
        assert_eq!(result.findings[0].line, 7);
    }

    // ── Positive e2e: a bad submit payload is recoverable — the model retries and succeeds ──────
    #[tokio::test]
    async fn native_loop_recovers_from_a_bad_submit() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![
                tool_call_reply("submit_findings", r#"{"findings":[]}"#), // missing summary
                tool_call_reply(
                    "submit_findings",
                    r#"{"summary":"All good.","findings":[]}"#,
                ),
            ],
        )
        .await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let result = run_native_review(&review, "review", None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect("review");
        assert_eq!(result.summary, "All good.");
        assert!(result.findings.is_empty());
    }

    // ── Negative e2e: abort surfaces as an error (recorded upstream, non-fatal to the task) ─────
    #[tokio::test]
    async fn native_loop_abort_is_an_error() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![tool_call_reply("abort", r#"{"reason":"diff unreadable"}"#)],
        )
        .await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let err = run_native_review(&review, "review", None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect_err("abort is an error");
        assert!(format!("{err:#}").contains("aborted"), "got: {err:#}");
    }

    // The ask prompt carries the question and (when present) the diff as context.
    #[test]
    fn build_ask_messages_carries_the_question() {
        let review = review_config("http://unused/v1".to_string());
        let msgs = build_ask_messages(&review, "why a mutex here?", None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        let user = msgs[1].content.as_deref().expect("user content");
        assert!(
            user.contains("why a mutex here?"),
            "question reaches prompt: {user}"
        );
    }

    // ── Positive e2e (ask): search → results → submit_answer → Markdown answer ──────────────────
    #[tokio::test]
    async fn native_ask_searches_then_answers() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![
                tool_call_reply(
                    "lightbridge_vector_semantic_search",
                    r#"{"query":"locking strategy"}"#,
                ),
                tool_call_reply(
                    "submit_answer",
                    r#"{"answer":"The mutex guards the shared cache; prefer an RwLock for read-heavy access."}"#,
                ),
            ],
        )
        .await;

        let emb = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "index": 0, "embedding": [0.1_f32, 0.2_f32] }]
            })))
            .mount(&emb)
            .await;
        let cp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/internal/tasks/{}/search", Uuid::nil())))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "file_path": "cache.rs", "language": "rust", "chunk_type": "function",
                "symbol_name": "get", "start_line": 1, "end_line": 9,
                "content": "fn get() {}", "score": 0.9
            }])))
            .mount(&cp)
            .await;

        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new(cp.uri(), "tok");
        let embc = EmbeddingsClient::new(&emb.uri(), "key", "model");
        let answer = run_native_ask(
            &review,
            "why a mutex here?",
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
        )
        .await
        .expect("answer");
        assert!(answer.contains("RwLock"), "got: {answer}");
    }

    // ── Negative e2e (ask): abort surfaces as an error ──────────────────────────────────────────
    #[tokio::test]
    async fn native_ask_abort_is_an_error() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![tool_call_reply("abort", r#"{"reason":"question unclear"}"#)],
        )
        .await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let err = run_native_ask(&review, "huh?", None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect_err("abort is an error");
        assert!(format!("{err:#}").contains("aborted"), "got: {err:#}");
    }

    // ── Negative e2e: a model that never submits is cut off by the turn budget ──────────────────
    #[tokio::test]
    async fn native_loop_gives_up_after_max_turns() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![text_reply("Hmm, let me think out loud forever.")],
        )
        .await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let err = run_native_review(&review, "review", None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect_err("should give up");
        assert!(
            format!("{err:#}").contains("did not finish"),
            "got: {err:#}"
        );
    }
}
