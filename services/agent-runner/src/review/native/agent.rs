//! The native agent loop (ADR-0026 + ADR-0037).
//!
//! Drives the [`ChatClient`] over the eaig gateway with the [`tool_defs`] surface: the system prompt
//! (the operator-owned persona + the machine tool-protocol) and the request → model → (retrieval +
//! mediated write actions) → … → `finish`. The agent **acts as it goes** — each `add_review_comment`
//! / `add_comment` is buffered control-plane-side; `finish` ends the run and the caller flushes the
//! buffer as one grouped review. The loop is bounded ([`MAX_TURNS`]) and treats a tool/argument error
//! as a recoverable turn (the model gets the error text back and retries), only failing on `abort`, an
//! exhausted turn budget, or a transport error. A run that fails this way is **never finalized**, so a
//! mid-run death posts nothing (crash-safe). Cancellation comes for free: `run()` races the whole task
//! against the self-cancel poll, so a cancelled task drops the in-flight future.

use anyhow::Context;
use uuid::Uuid;

use super::chat::{ChatClient, ChatMessage, ChatParams};
use super::tools::{tool_defs, ToolOutcome, Tools, ADD_REVIEW_COMMENT};
use crate::bootstrap::client::ControlPlaneClient;
use crate::bootstrap::config::ReviewConfig;
use crate::clone::PrDiff;
use crate::indexer::embeddings::EmbeddingsClient;

/// Hard ceiling on model turns, so a model that never finishes can't loop forever (each turn is one
/// chat round-trip; tool calls within a turn don't count separately).
const MAX_TURNS: usize = 16;

/// The machine **tool-protocol** appended after the operator's system prompt (ADR-0037). This is the
/// only behaviour-shaping text that lives in code — it is factual and coupled to the tool API (names,
/// when to call them), NOT persona/guidance, which is operator-owned config (`review.system_prompt`,
/// from the ai-helm `config.reviewSystemPrompt`). It goes last so it's the final instruction.
const TOOL_PROTOCOL: &str = "\
# How to act\n\
Investigate with the search/graph tools before making any claim — never speculate about code you \
have not looked up. As you find issues, record each one with `add_review_comment` (one call per \
finding, on a line this diff adds or changes). Use `add_comment` for a plain reply that isn't pinned \
to a diff line (e.g. answering a question). Nothing you record is posted until you call `finish` with \
your overall verdict — call `finish` exactly once when you are done, even if you found nothing. If you \
genuinely cannot produce anything useful, call `abort` with a reason. You may not edit files or run \
commands.";

/// Run the native agent loop. The agent acts via the mediated write tools during the run; on a clean
/// `finish` this returns `Ok(())` and the caller flushes the buffer (`finalize_review`). Any error
/// (abort, exhausted budget, transport) returns `Err` and the caller does **not** finalize — so a
/// failed/partial run posts nothing.
#[allow(clippy::too_many_arguments)]
pub async fn run_native_agent(
    review: &ReviewConfig,
    command: &str,
    diff: Option<&PrDiff>,
    repo_instructions: Option<&str>,
    attribution: &[(String, String)],
    client: &ControlPlaneClient,
    embedder: &EmbeddingsClient,
    task_id: Uuid,
) -> anyhow::Result<()> {
    let chat = ChatClient::new(&review.base_url, &review.api_key, &review.model)
        .with_attribution(attribution);
    let tools = Tools {
        client,
        embedder,
        task_id,
    };
    // Without a diff (an issue target, or `git diff` was unavailable) an inline finding has no line to
    // anchor to — finalize would only bucket it. Don't offer `add_review_comment` then, so the model
    // replies via `add_comment` instead of hallucinating inline comments that go nowhere.
    let mut defs = tool_defs();
    if diff.is_none() {
        defs.retain(|t| t.function.name != ADD_REVIEW_COMMENT);
    }
    let params = ChatParams {
        temperature: review.temperature,
        top_p: review.top_p,
        max_tokens: review.max_tokens,
    };

    let mut messages = build_messages(review, command, diff, repo_instructions);

    for turn in 0..MAX_TURNS {
        let completion = chat
            .complete(&messages, &defs, params)
            .await
            .with_context(|| format!("agent chat turn {turn}"))?;
        let assistant = completion.message;
        let calls = assistant.tool_calls.clone();
        // Echo the assistant turn (with its tool_calls) back into the conversation, as the protocol
        // requires before the matching tool-result messages.
        messages.push(assistant);

        if calls.is_empty() {
            // The model replied in prose instead of acting — steer it back to the tools.
            messages.push(ChatMessage::user(
                "Use the tools to investigate and record findings with `add_review_comment` (or a \
                 reply with `add_comment`), then call `finish` with your verdict (or `abort`). Do \
                 not reply in prose.",
            ));
            continue;
        }

        // Dispatch every call in the turn before acting on a terminal outcome: a model may emit
        // parallel tool calls (e.g. a last `add_review_comment` alongside `finish`), and we must not
        // drop the others just because `finish` appeared first. Each dispatch still runs its side
        // effect (the write tools buffer immediately); we only defer the loop-control decision.
        let mut should_finish = false;
        let mut abort_reason = None;
        for call in &calls {
            match tools.dispatch(call).await {
                ToolOutcome::Finish => should_finish = true,
                ToolOutcome::Abort(reason) => abort_reason = Some(reason),
                ToolOutcome::Continue(result) => {
                    messages.push(ChatMessage::tool(call.id.as_str(), result));
                }
            }
        }
        // Abort wins over finish if the model somehow asked for both — it's the safer outcome (a
        // failed run posts nothing).
        if let Some(reason) = abort_reason {
            anyhow::bail!("review agent aborted: {reason}");
        }
        if should_finish {
            return Ok(());
        }
    }

    anyhow::bail!("review agent did not finish within {MAX_TURNS} turns")
}

/// Assemble the system (operator prompt + tool-protocol) and user (request + diff) messages. The
/// system prompt is the **required** operator-owned guidance (ADR-0037 — no built-in default); the
/// tool-protocol is appended last so it's the final instruction the model sees.
fn build_messages(
    review: &ReviewConfig,
    command: &str,
    diff: Option<&PrDiff>,
    repo_instructions: Option<&str>,
) -> Vec<ChatMessage> {
    let system = format!("{}\n\n{TOOL_PROTOCOL}", review.system_prompt);

    let mut user = format!("The maintainer's request: {command}");
    match diff {
        Some(pr) => {
            user.push_str(&format!(
                "\n\nThis pull request changes {} file(s):\n{}",
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
            "\n\nNo diff is available for this run; answer or review against the working tree and \
             keep every claim grounded in the tools.",
        ),
    }

    // Repo-native agent instructions (ADR-0036), kept in the user message as untrusted context (it is
    // already labelled and the tool-protocol/mission in the system message stays authoritative).
    if let Some(instructions) = repo_instructions {
        user.push_str("\n\n");
        user.push_str(instructions);
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
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// A scripted chat endpoint: returns `responses[i]` on the i-th call, repeating the last forever
    /// (so a loop that never finishes keeps getting the same reply).
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
            base_url: chat_base_with_v1,
            api_key: "k".to_string(),
            model: "m".to_string(),
            system_prompt: "You are a reviewer.".to_string(),
            max_diff_chars: 60_000,
            temperature: None,
            top_p: None,
            max_tokens: None,
        }
    }

    // The maintainer's request reaches the user prompt; the operator system prompt is used verbatim.
    #[test]
    fn build_messages_carries_request_and_uses_operator_prompt() {
        let review = review_config("http://unused/v1".to_string());
        let msgs = build_messages(&review, "propose a better implementation", None, None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        let system = msgs[0].content.as_deref().expect("system content");
        assert!(
            system.starts_with("You are a reviewer."),
            "operator prompt first"
        );
        assert!(system.contains("How to act"), "tool-protocol appended");
        let user = msgs[1].content.as_deref().expect("user content");
        assert!(
            user.contains("propose a better implementation"),
            "request reaches prompt: {user}"
        );
    }

    // ── Positive e2e: search → add_review_comment → finish → Ok(()) ─────────────────────────────
    #[tokio::test]
    async fn native_loop_searches_records_and_finishes() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![
                tool_call_reply(
                    "lightbridge_vector_semantic_search",
                    r#"{"query":"session expiry"}"#,
                ),
                tool_call_reply(
                    "add_review_comment",
                    r#"{"file":"a.rs","line":7,"title":"No expiry","priority":"P0","category":"security","body":"accepts expired tokens"}"#,
                ),
                tool_call_reply("finish", r#"{"summary":"Missing expiry check."}"#),
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
                "file_path": "a.rs", "language": "rust", "chunk_type": "function",
                "symbol_name": "validate", "start_line": 1, "end_line": 9,
                "content": "fn validate() {}", "score": 0.9
            }])))
            .mount(&cp)
            .await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/internal/tasks/{}/review/inline",
                Uuid::nil()
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&cp)
            .await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/internal/tasks/{}/review/summary",
                Uuid::nil()
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&cp)
            .await;

        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new(cp.uri(), "tok");
        let embc = EmbeddingsClient::new(&emb.uri(), "key", "model");
        // A diff is present, so `add_review_comment` is offered.
        let diff = PrDiff {
            diff: "@@ -1,3 +1,4 @@\n fn validate() {}\n+// changed\n".to_string(),
            files: vec!["a.rs".to_string()],
        };
        run_native_agent(
            &review,
            "@lightbridge review",
            Some(&diff),
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
        )
        .await
        .expect("agent finishes cleanly");
    }

    // No diff → `add_review_comment` is not offered (an inline finding can't anchor); the rest of the
    // tool surface remains.
    #[test]
    fn no_diff_omits_add_review_comment_from_offered_tools() {
        let offered: Vec<String> = {
            let mut defs = tool_defs();
            defs.retain(|t| t.function.name != ADD_REVIEW_COMMENT);
            defs.iter().map(|t| t.function.name.clone()).collect()
        };
        assert!(!offered.iter().any(|n| n == ADD_REVIEW_COMMENT));
        assert!(
            offered.iter().any(|n| n == "add_comment"),
            "add_comment still offered"
        );
        assert!(offered.iter().any(|n| n == "finish"));
    }

    // ── Negative e2e: abort surfaces as an error (caller does not finalize) ─────────────────────
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
        let err = run_native_agent(&review, "review", None, None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect_err("abort is an error");
        assert!(format!("{err:#}").contains("aborted"), "got: {err:#}");
    }

    // ── Negative e2e: a model that never finishes is cut off by the turn budget ─────────────────
    #[tokio::test]
    async fn native_loop_gives_up_after_max_turns() {
        let chat = MockServer::start().await;
        mount_chat(&chat, vec![text_reply("Thinking out loud forever.")]).await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let err = run_native_agent(&review, "review", None, None, &[], &cpc, &embc, Uuid::nil())
            .await
            .expect_err("should give up");
        assert!(
            format!("{err:#}").contains("did not finish"),
            "got: {err:#}"
        );
    }
}
