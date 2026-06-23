//! The native agent loop (ADR-0026 + ADR-0037).
//!
//! Drives the [`ChatClient`] over the eaig gateway with the [`tool_defs`] surface: the system prompt
//! (the operator-owned persona + the machine tool-protocol) and the request → model → (retrieval +
//! mediated write actions) → … → `finish`. The agent **acts as it goes** — each `add_review_comment`
//! / `add_comment` is buffered control-plane-side; `finish` ends the run and the caller flushes the
//! buffer as one grouped review. The loop is bounded (`review.max_turns`) and treats a tool/argument
//! error as a recoverable turn (the model gets the error text back and retries).
//!
//! Outcome model (#137): the loop returns a [`ReviewOutcome`] — `Finished` (the model called `finish`),
//! `Exhausted` (the turn budget ran out while findings may be buffered), or `Aborted(reason)` (the
//! model called `abort`). **Only a true transport/chat failure returns `Err`.** This is the key
//! behavioural fix: the buffer (which the control plane holds) is NEVER discarded on a clean exhaustion
//! — the caller finalizes on `Finished` OR `Exhausted` so buffered findings are still posted, with a
//! truncation note on `Exhausted` and an abort note on `Aborted`. A transport layer (ADR-0039) wraps
//! each turn with a generous timeout, bounded retry/backoff on transient failures, optional failover to
//! a secondary model, and a per-run circuit breaker; the HTTP error body is folded into the error so a
//! failed run is legible. Cancellation comes for free: `run()` races the whole task against the
//! self-cancel poll, so a cancelled task drops the in-flight future.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use uuid::Uuid;

use super::chat::{ChatClient, ChatMessage, ChatParams, RetryPolicy, ToolDef};
use super::tools::{tool_defs, ToolOutcome, Tools, ABORT, ADD_COMMENT, ADD_REVIEW_COMMENT, FINISH};
use crate::bootstrap::client::{ControlPlaneClient, TranscriptEntry};
use crate::bootstrap::config::ReviewConfig;
use crate::clone::PrDiff;
use crate::indexer::embeddings::EmbeddingsClient;

/// How the agent loop ended (#137). Distinct from `Err`, which is reserved for a transport/chat
/// failure where the gateway was unreachable and nothing useful happened. The caller maps these to a
/// visible artifact on the PR:
/// - [`ReviewOutcome::Finished`] — the model called `finish`; finalize flushes the buffer.
/// - [`ReviewOutcome::Exhausted`] — the turn budget ran out with findings possibly still buffered;
///   the caller posts a truncation note then finalizes (so buffered findings are NOT discarded).
/// - [`ReviewOutcome::Aborted`] — the model called `abort`; the caller posts the reason then finalizes.
#[derive(Debug)]
pub enum ReviewOutcome {
    Finished,
    Exhausted,
    Aborted(String),
}

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

/// Wind-down convergence (#137): how many turns before the budget ceiling we switch the model onto a
/// reduced tool set so it stops investigating and converges to a `finish`. Computed per-run as a
/// fraction of `max_turns` (see [`winddown_turn`]) so a tiny budget still leaves a turn or two to wrap
/// up and a generous budget gets a proportional tail. Observed in prod (izhub#205/#207): the agent ran
/// the full budget with **0** `finish` calls — endless retrieval/`read_file` rabbit holes — and the
/// `Exhausted` safety-net fired instead of a real verdict. The lever is removing the investigation
/// tools near the end: with only the write/finish/abort tools left, the model must record any last
/// findings and finish.
const WINDDOWN_MIN_TURNS: usize = 2;

/// The first turn index at which the wind-down (reduced tool set + budget message) kicks in. Reserves
/// `max(WINDDOWN_MIN_TURNS, max_turns / 10)` turns at the tail of the budget, clamped so it never lands
/// before turn 1 (we always allow at least one full-toolset turn) nor at/after the ceiling.
fn winddown_turn(max_turns: usize) -> usize {
    if max_turns <= WINDDOWN_MIN_TURNS {
        // Tiny budgets: wind down on the final turn so there's still one investigation turn first.
        return max_turns.saturating_sub(1).max(1);
    }
    let reserve = WINDDOWN_MIN_TURNS.max(max_turns / 10);
    // Keep at least one full-toolset turn at the start, and at least one wind-down turn at the end.
    max_turns
        .saturating_sub(reserve)
        .clamp(1, max_turns.saturating_sub(1))
}

/// The reduced tool set offered once the run enters its wind-down (#137): the write tools (so the model
/// can record any last findings), plus `finish`/`abort` (so it can converge). **Drops** the retrieval
/// tools, `read_file`, and `report_progress` — with no way to keep investigating, the model must wrap
/// up. `add_review_comment` is only kept when a diff is present (mirrors the full-set gating, so a
/// no-diff run never offers an inline tool that can't anchor).
fn winddown_tool_defs(diff_present: bool) -> Vec<ToolDef> {
    tool_defs()
        .into_iter()
        .filter(|t| {
            let name = t.function.name.as_str();
            match name {
                ADD_REVIEW_COMMENT => diff_present,
                ADD_COMMENT | FINISH | ABORT => true,
                _ => false,
            }
        })
        .collect()
}

/// Run the native agent loop. The agent acts via the mediated write tools during the run. Returns a
/// [`ReviewOutcome`] describing how it ended (`Finished` / `Exhausted` / `Aborted`) — the caller turns
/// each into a visible PR artifact and finalizes the buffer in all three cases (#137). Only a true
/// transport/chat failure returns `Err`; in that case nothing is posted.
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
    // The checked-out repo root (the working tree under review). `read_file` reads from here,
    // path-sanitized to within it, so the model can open the actual source (epic #137).
    checkout_root: &Path,
    // Accumulates the run transcript (ADR-0034) as the loop progresses. The caller owns it and submits
    // it afterwards (even on error), so a failed run's reasoning is still captured.
    transcript: &mut Vec<TranscriptEntry>,
) -> anyhow::Result<ReviewOutcome> {
    let chat = ChatClient::with_timeout(
        &review.base_url,
        &review.api_key,
        &review.model,
        Duration::from_secs(review.resilience.request_timeout_secs),
    )
    .with_attribution(attribution);
    // Optional secondary model for failover (ADR-0039): same gateway/key, different model id.
    let fallback = review
        .resilience
        .fallback_model
        .as_deref()
        .map(|m| chat.for_model(m));
    let retry_policy = RetryPolicy {
        max_retries: review.resilience.max_retries,
        ..RetryPolicy::default()
    };
    // Proof-of-work (ADR-0039): a run is legible from pod logs alone — which model, which gateway, and
    // the resilience policy in force. host-only for the base URL (the path/key are uninteresting/secret).
    tracing::info!(
        task_id = %task_id,
        model = %review.model,
        fallback_model = review.resilience.fallback_model.as_deref().unwrap_or("(none)"),
        base_url_host = %base_url_host(&review.base_url),
        request_timeout_secs = review.resilience.request_timeout_secs,
        max_retries = review.resilience.max_retries,
        circuit_breaker_threshold = review.resilience.circuit_breaker_threshold,
        "review agent starting"
    );

    let tools = Tools {
        client,
        embedder,
        task_id,
        checkout_root,
    };
    // Without a diff (an issue target, or `git diff` was unavailable) an inline finding has no line to
    // anchor to — finalize would only bucket it. Don't offer `add_review_comment` then, so the model
    // replies via `add_comment` instead of hallucinating inline comments that go nowhere.
    let diff_present = diff.is_some();
    let mut defs = tool_defs();
    if !diff_present {
        defs.retain(|t| t.function.name != ADD_REVIEW_COMMENT);
    }
    // Reduced tool set for the wind-down tail (#137): write/finish/abort only, retrieval/read_file
    // dropped so the model can no longer keep investigating once the budget is nearly spent.
    let winddown_defs = winddown_tool_defs(diff_present);
    let params = ChatParams {
        temperature: review.temperature,
        top_p: review.top_p,
        max_tokens: review.max_tokens,
    };

    let mut messages = build_messages(review, command, diff, repo_instructions);

    // Per-run circuit breaker (ADR-0039): consecutive turn-failures. The Job is ephemeral, so this is
    // deliberately per-process — no cross-process state. Resets on the first turn that succeeds.
    let mut consecutive_failures = 0u32;
    let breaker_threshold = review.resilience.circuit_breaker_threshold;

    // Operator-tunable turn budget (#137): a tight ceiling used to exhaust mid-PR with findings still
    // buffered. Generous default lives in config; a turn is ~6s on the deepseek model.
    let max_turns = review.max_turns;
    // Once the model has recorded ≥1 finding we nudge it to call `finish` so a wandering run doesn't burn
    // the budget after the useful work is already buffered (#137). Nudge once, lightly.
    let mut findings_recorded = 0usize;
    let mut nudged_to_finish = false;
    // Wind-down convergence (#137): the turn at which we restrict the tool set + inject the budget
    // message, plus a one-time soft nudge around the halfway mark. The tool restriction is the real
    // lever — the messages just explain why the tools changed.
    let winddown = winddown_turn(max_turns);
    let halfway = max_turns / 2;
    let mut winddown_announced = false;
    let mut halfway_nudged = false;

    for turn in 0..max_turns {
        let turn_started = Instant::now();

        // Wind-down (#137): as the budget depletes, switch the model onto the reduced tool set and tell
        // it (once) to stop investigating and converge. The reduced set drops retrieval/read_file, so
        // the model has no way to keep digging — it must record any last findings and `finish`. The
        // existing `Exhausted` path below stays as the ultimate backstop if it STILL doesn't finish.
        let in_winddown = turn >= winddown;
        let turn_defs: &[ToolDef] = if in_winddown { &winddown_defs } else { &defs };
        if in_winddown && !winddown_announced {
            winddown_announced = true;
            messages.push(ChatMessage::user(format!(
                "⏳ Turn budget almost spent (turn {turn}/{max_turns}). Stop investigating — record \
                 any remaining findings now with add_review_comment/add_comment, then call `finish` \
                 with your overall verdict. (The investigation tools are no longer available.)"
            )));
        } else if !halfway_nudged && halfway > 0 && turn >= halfway {
            // Softer one-time nudge around the halfway mark — keep it light; the tool restriction at
            // the wind-down boundary is the real lever.
            halfway_nudged = true;
            messages.push(ChatMessage::user(
                "You're past halfway on your turn budget — start converging: record what you've found \
                 and head toward `finish`.",
            ));
        }

        // Try the primary model with bounded retry; on exhausting transient retries, fail over to the
        // secondary model (when configured) once for this turn. The outcome is either a completion, or
        // a turn-level error we classify before deciding whether to keep going.
        let turn_result = match chat
            .complete_with_retry(&messages, turn_defs, params, retry_policy)
            .await
        {
            Ok(c) => Ok(c),
            Err(primary_err) => match (&fallback, primary_err.transient) {
                (Some(fb), true) => {
                    tracing::warn!(
                        task_id = %task_id,
                        turn,
                        primary_model = %review.model,
                        fallback_model = %fb.model(),
                        error = %primary_err,
                        "primary model exhausted retries; failing over to secondary model"
                    );
                    fb.complete_with_retry(&messages, turn_defs, params, retry_policy)
                        .await
                        .map_err(|fb_err| ChatTurnError {
                            error: fb_err
                                .error
                                .context(format!("failover model {} also failed", fb.model())),
                            transient: fb_err.transient,
                        })
                }
                _ => Err(ChatTurnError {
                    error: primary_err.error,
                    transient: primary_err.transient,
                }),
            },
        };

        let completion = match turn_result {
            Ok(c) => {
                // The turn produced a model reply → the chain is healthy again.
                consecutive_failures = 0;
                c
            }
            Err(turn_err) => {
                // A deterministic failure (4xx other than 429, a malformed body) won't get better by
                // trying again — fail the run now with the legible reason. A transient failure counts
                // toward the per-run circuit breaker: keep going until it trips or the budget runs out,
                // rather than wasting the whole budget against a chain that's clearly down.
                if !turn_err.transient {
                    return Err(turn_err.error).with_context(|| format!("agent chat turn {turn}"));
                }
                consecutive_failures += 1;
                tracing::warn!(
                    task_id = %task_id,
                    turn,
                    consecutive_failures,
                    breaker_threshold,
                    error = %turn_err.error,
                    "transient turn failure after retries (and failover, if configured)"
                );
                if breaker_threshold > 0 && consecutive_failures >= breaker_threshold {
                    return Err(turn_err.error).with_context(|| {
                        format!(
                            "review agent circuit breaker tripped after {consecutive_failures} \
                             consecutive turn failures (threshold {breaker_threshold}); failing fast \
                             at turn {turn}"
                        )
                    });
                }
                continue;
            }
        };
        let turn_latency_ms = turn_started.elapsed().as_millis() as u64;

        let usage = completion.usage;
        let assistant = completion.message;
        let calls = assistant.tool_calls.clone();

        // One concise line per turn (ADR-0034/0039): index, tools called, tokens, wall-clock latency.
        // Full content lives in the transcript; this keeps pod logs legible without the payloads.
        let tool_names: Vec<&str> = calls.iter().map(|c| c.function.name.as_str()).collect();
        tracing::info!(
            task_id = %task_id,
            turn,
            tools = ?tool_names,
            prompt_tokens = usage.and_then(|u| u.prompt_tokens).unwrap_or(-1),
            completion_tokens = usage.and_then(|u| u.completion_tokens).unwrap_or(-1),
            latency_ms = turn_latency_ms,
            "agent turn complete"
        );
        // Proof-of-work (epic #137): log the model's reasoning for this turn (bounded) so a run is
        // legible from a live log tail, not just the DB transcript. Skip when the turn was pure
        // tool-calls with no prose.
        if let Some(reasoning) = assistant
            .content
            .as_deref()
            .filter(|c| !c.trim().is_empty())
        {
            tracing::info!(
                task_id = %task_id,
                turn,
                reasoning = %truncate_on_boundary(reasoning, 600),
                "agent reasoning"
            );
        }
        // Record the assistant turn in the transcript (ADR-0034): its reasoning + tool calls + tokens.
        transcript.push(TranscriptEntry {
            role: "assistant".to_string(),
            content: assistant.content.clone(),
            tool_calls: (!calls.is_empty())
                .then(|| serde_json::to_value(&calls).unwrap_or_default()),
            tool_name: None,
            prompt_tokens: usage.and_then(|u| u.prompt_tokens),
            completion_tokens: usage.and_then(|u| u.completion_tokens),
        });
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
            let tool = call.function.name.as_str();
            // One concise line per tool dispatch with the (bounded) call arguments — so a live log
            // tail shows what the model actually asked for, not just the tool name (epic #137). For
            // the mediated write tools, note the buffer effect.
            let args = truncate_on_boundary(&call.function.arguments, 400);
            match tool {
                ADD_REVIEW_COMMENT | "add_comment" => tracing::info!(
                    task_id = %task_id, turn, tool, args = %args, "tool dispatch (finding/reply buffered)"
                ),
                _ => tracing::info!(task_id = %task_id, turn, tool, args = %args, "tool dispatch"),
            }
            match tools.dispatch(call).await {
                ToolOutcome::Finish => should_finish = true,
                ToolOutcome::Abort(reason) => abort_reason = Some(reason),
                ToolOutcome::Continue(result) => {
                    // Count successful inline findings so we know when to nudge the model toward
                    // `finish` (and only count the ones the control plane actually buffered).
                    if tool == ADD_REVIEW_COMMENT && result.starts_with("recorded finding") {
                        findings_recorded += 1;
                    }
                    // Proof-of-work (epic #137): a result summary so empty retrievals are visible in
                    // the live log stream (the full result still goes to the DB transcript below).
                    tracing::info!(
                        task_id = %task_id,
                        turn,
                        tool,
                        result_len = result.len(),
                        result = %result_summary(&result),
                        "tool result"
                    );
                    // Record the tool result in the transcript (bounded), then feed it back to the model.
                    transcript.push(TranscriptEntry {
                        role: "tool".to_string(),
                        content: Some(truncate_on_boundary(&result, 2048).to_string()),
                        tool_calls: None,
                        tool_name: Some(call.function.name.clone()),
                        prompt_tokens: None,
                        completion_tokens: None,
                    });
                    messages.push(ChatMessage::tool(call.id.as_str(), result));
                }
            }
        }
        // Abort wins over finish if the model somehow asked for both — it's the safer signal.
        if let Some(reason) = abort_reason {
            return Ok(ReviewOutcome::Aborted(reason));
        }
        if should_finish {
            return Ok(ReviewOutcome::Finished);
        }

        // Light nudge (#137): once useful work is buffered, remind the model to wrap up with `finish`
        // so it doesn't wander and exhaust the budget after the findings are already recorded. Once.
        if findings_recorded > 0 && !nudged_to_finish {
            nudged_to_finish = true;
            messages.push(ChatMessage::user(
                "You have recorded at least one finding. When your investigation is complete, call \
                 `finish` with your overall verdict to post everything you've buffered — don't keep \
                 investigating past the point of useful work.",
            ));
        }
    }

    // Turn budget exhausted. CRITICAL (#137): do NOT bail — that would discard the buffered findings
    // the control plane is holding. Return `Exhausted` so the caller posts a truncation note and
    // finalizes, leaving a visible artifact (a real prod run lost 5 findings this way at turn 16).
    tracing::warn!(
        task_id = %task_id,
        max_turns,
        findings_recorded,
        "review agent hit its turn budget without calling finish — finalizing buffered findings"
    );
    Ok(ReviewOutcome::Exhausted)
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

/// A turn-level chat failure after retries (and failover, if configured), carrying whether the
/// underlying error was transient — the loop keeps a transient one going toward the circuit breaker
/// but fails the run immediately on a deterministic one.
struct ChatTurnError {
    error: anyhow::Error,
    transient: bool,
}

/// The host of a base URL for logging — keeps the path/query (and any embedded token) out of logs
/// while still identifying which gateway a run hit. Falls back to a redacted marker if unparseable.
fn base_url_host(base_url: &str) -> String {
    // Fall back to the whole string when there's no scheme separator (e.g. `gateway.example/v1`),
    // so a schemeless URL still logs its host rather than "(unparseable)".
    let without_scheme = base_url.split("://").nth(1).unwrap_or(base_url);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .map(|hostport| hostport.to_string())
        .unwrap_or_else(|| "(unparseable)".to_string())
}

/// A short, log-friendly summary of a tool result for live tailing (epic #137). Calls out an empty
/// retrieval explicitly (a pretty-printed empty JSON array renders as `[]`) — the common failure the
/// model used to flail against blindly — and otherwise returns the leading bytes (bounded).
fn result_summary(result: &str) -> String {
    let trimmed = result.trim();
    if trimmed == "[]" {
        return "0 hits".to_string();
    }
    truncate_on_boundary(trimmed, 200).to_string()
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
            max_turns: crate::bootstrap::config::DEFAULT_MAX_TURNS,
            temperature: None,
            top_p: None,
            max_tokens: None,
            // Fast resilience defaults so the loop tests don't sleep on the (mocked) failure paths.
            resilience: crate::bootstrap::config::ResilienceConfig {
                request_timeout_secs: 5,
                max_retries: 0,
                circuit_breaker_threshold: 3,
                fallback_model: None,
            },
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
        let mut transcript = Vec::new();
        let outcome = run_native_agent(
            &review,
            "@lightbridge review",
            Some(&diff),
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut transcript,
        )
        .await
        .expect("agent finishes cleanly");
        assert!(
            matches!(outcome, ReviewOutcome::Finished),
            "got: {outcome:?}"
        );
        // The transcript captured the assistant turns (search → add_review_comment → finish).
        assert!(
            transcript.iter().any(|e| e.role == "assistant"),
            "transcript records assistant turns"
        );
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

    // ── e2e: abort returns ReviewOutcome::Aborted(reason) — NOT an Err (#137). The caller posts the
    // reason as the review summary then finalizes, so the PR gets an honest abort note, not silence. ──
    #[tokio::test]
    async fn native_loop_abort_returns_aborted_outcome() {
        let chat = MockServer::start().await;
        mount_chat(
            &chat,
            vec![tool_call_reply("abort", r#"{"reason":"diff unreadable"}"#)],
        )
        .await;
        let review = review_config(format!("{}/v1", chat.uri()));
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let outcome = run_native_agent(
            &review,
            "review",
            None,
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut Vec::new(),
        )
        .await
        .expect("abort is a clean outcome, not an Err");
        match outcome {
            ReviewOutcome::Aborted(reason) => assert_eq!(reason, "diff unreadable"),
            other => panic!("expected Aborted, got {other:?}"),
        }
    }

    // ── Failover: the primary model 5xx's; the configured secondary model handles the turn and the
    // run finishes cleanly (ADR-0039). Mocks are matched on the request body's `model` field. ──────
    #[tokio::test]
    async fn native_loop_fails_over_to_secondary_model() {
        use wiremock::matchers::body_partial_json;

        let chat = MockServer::start().await;
        // Primary model "m" always 5xx (transient → retries exhaust → failover).
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({ "model": "m" })))
            .respond_with(ResponseTemplate::new(503))
            .mount(&chat)
            .await;
        // Secondary model "m-fallback" finishes immediately.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({ "model": "m-fallback" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(tool_call_reply(
                "finish",
                r#"{"summary":"handled by fallback"}"#,
            )))
            .mount(&chat)
            .await;

        // No diff → no inline finalize; only the summary endpoint is hit on finish.
        let cp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/internal/tasks/{}/review/summary",
                Uuid::nil()
            )))
            .respond_with(ResponseTemplate::new(204))
            .mount(&cp)
            .await;

        let mut review = review_config(format!("{}/v1", chat.uri()));
        review.resilience.fallback_model = Some("m-fallback".to_string());
        let cpc = ControlPlaneClient::new(cp.uri(), "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        run_native_agent(
            &review,
            "review",
            None,
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut Vec::new(),
        )
        .await
        .expect("fallback model finishes the run");
    }

    // ── Circuit breaker: the chain is down (persistent 5xx) and no fallback is set, so the run fails
    // fast at the breaker threshold instead of consuming the whole turn budget (ADR-0039). ─────────
    #[tokio::test]
    async fn native_loop_circuit_breaker_trips_fast() {
        let chat = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&chat)
            .await;

        let mut review = review_config(format!("{}/v1", chat.uri()));
        review.resilience.circuit_breaker_threshold = 1; // trip on the first failure
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let err = run_native_agent(
            &review,
            "review",
            None,
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut Vec::new(),
        )
        .await
        .expect_err("breaker trips");
        assert!(
            format!("{err:#}").contains("circuit breaker tripped"),
            "got: {err:#}"
        );
    }

    // ── e2e: a model that never finishes is cut off by the turn budget and returns Exhausted — NOT an
    // Err (#137). The buffered findings the control plane holds must survive so the caller can finalize
    // (a real prod run lost 5 findings when this used to bail). ──────────────────────────────────────
    #[tokio::test]
    async fn native_loop_exhausts_budget_without_discarding() {
        let chat = MockServer::start().await;
        mount_chat(&chat, vec![text_reply("Thinking out loud forever.")]).await;
        let mut review = review_config(format!("{}/v1", chat.uri()));
        review.max_turns = 3; // keep the test fast; the model never calls finish
        let cpc = ControlPlaneClient::new("http://unused", "tok");
        let embc = EmbeddingsClient::new("http://unused", "key", "model");
        let outcome = run_native_agent(
            &review,
            "review",
            None,
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut Vec::new(),
        )
        .await
        .expect("exhaustion is a clean outcome, not an Err");
        assert!(
            matches!(outcome, ReviewOutcome::Exhausted),
            "got: {outcome:?}"
        );
    }

    // ── Wind-down helpers (#137): the boundary reserves a proportional tail and never lands before the
    // first turn or at/after the ceiling; the reduced set keeps only write/finish/abort. ────────────
    #[test]
    fn winddown_turn_reserves_a_proportional_tail() {
        // Generous budget: reserve max(2, 150/10) = 15 → wind down at 135.
        assert_eq!(winddown_turn(150), 135);
        // Mid budget: reserve max(2, 40/10) = 4 → wind down at 36.
        assert_eq!(winddown_turn(40), 36);
        // Small budget: reserve max(2, 5/10)=2 → wind down at 3 (turns 3,4 reduced; 0,1,2 full).
        assert_eq!(winddown_turn(5), 3);
        // Tiny budgets still leave one full-toolset turn first and never land at/after the ceiling.
        assert_eq!(winddown_turn(2), 1);
        assert_eq!(winddown_turn(1), 1);
        assert!(winddown_turn(0) >= 1);
    }

    #[test]
    fn winddown_tool_defs_drops_investigation_tools() {
        let names: Vec<String> = winddown_tool_defs(true)
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        // Kept: the write tools + finish + abort.
        for kept in [ADD_REVIEW_COMMENT, ADD_COMMENT, FINISH, ABORT] {
            assert!(names.iter().any(|n| n == kept), "{kept} should be kept");
        }
        // Dropped: every investigation/progress tool.
        for dropped in [
            super::super::tools::VECTOR_SEMANTIC_SEARCH,
            super::super::tools::GRAPH_FIND_SYMBOL,
            super::super::tools::GRAPH_GET_CALLERS,
            super::super::tools::READ_FILE,
            super::super::tools::REPORT_PROGRESS,
        ] {
            assert!(
                !names.iter().any(|n| n == dropped),
                "{dropped} should be dropped in wind-down"
            );
        }
        // No diff → `add_review_comment` is not even offered in the reduced set.
        let no_diff: Vec<String> = winddown_tool_defs(false)
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        assert!(!no_diff.iter().any(|n| n == ADD_REVIEW_COMMENT));
        assert!(no_diff.iter().any(|n| n == ADD_COMMENT));
    }

    /// A scripted chat endpoint that also records, per call, the tool names offered in the request body
    /// `tools` array — so a test can assert the wind-down restricted the surface on the late turns.
    struct RecordingScript {
        calls: Arc<AtomicUsize>,
        offered: Arc<std::sync::Mutex<Vec<Vec<String>>>>,
        // The concatenated user-message text seen in each request body (lets a test assert the budget
        // message was injected into the conversation by the time the wind-down turn fires).
        user_text: Arc<std::sync::Mutex<Vec<String>>>,
        response: serde_json::Value,
    }
    impl Respond for RecordingScript {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let names = body
                .get("tools")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| {
                            t.get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|n| n.as_str())
                                .map(str::to_string)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            self.offered.lock().unwrap().push(names);
            let user_text = body
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            self.user_text.lock().unwrap().push(user_text);
            ResponseTemplate::new(200).set_body_json(self.response.clone())
        }
    }

    // ── e2e wind-down (#137): a model that keeps requesting retrieval and never finishes. Once the
    // wind-down turn is reached the offered tool set must drop the retrieval/read_file tools (only
    // write/finish/abort remain), and a budget message must have been injected. The run still ends in
    // `Exhausted` (the backstop) because this model never calls `finish`. ────────────────────────────
    #[tokio::test]
    async fn native_loop_restricts_tools_in_winddown() {
        let chat = MockServer::start().await;
        let offered = Arc::new(std::sync::Mutex::new(Vec::new()));
        let user_text = Arc::new(std::sync::Mutex::new(Vec::new()));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(RecordingScript {
                calls: Arc::new(AtomicUsize::new(0)),
                offered: offered.clone(),
                user_text: user_text.clone(),
                // Always asks for retrieval; never finishes.
                response: tool_call_reply(
                    "lightbridge_vector_semantic_search",
                    r#"{"query":"anything"}"#,
                ),
            })
            .mount(&chat)
            .await;

        // The control plane only needs to answer the search calls (no finalize — the loop never
        // finishes; the caller would finalize, not the loop). Return 0 hits each time.
        let cp = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/internal/tasks/{}/search", Uuid::nil())))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&cp)
            .await;
        let emb = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{ "index": 0, "embedding": [0.1_f32, 0.2_f32] }]
            })))
            .mount(&emb)
            .await;

        let mut review = review_config(format!("{}/v1", chat.uri()));
        review.max_turns = 5; // winddown_turn(5) == 3 → turns 0,1,2 full; 3,4 reduced.
        let cpc = ControlPlaneClient::new(cp.uri(), "tok");
        let embc = EmbeddingsClient::new(&emb.uri(), "key", "model");
        // A diff is present so `add_review_comment` is part of the (reduced) set.
        let diff = PrDiff {
            diff: "@@ -1,1 +1,2 @@\n fn x() {}\n+// changed\n".to_string(),
            files: vec!["a.rs".to_string()],
        };
        let outcome = run_native_agent(
            &review,
            "review",
            Some(&diff),
            None,
            &[],
            &cpc,
            &embc,
            Uuid::nil(),
            std::path::Path::new("/tmp"),
            &mut Vec::new(),
        )
        .await
        .expect("exhaustion is a clean outcome");
        assert!(
            matches!(outcome, ReviewOutcome::Exhausted),
            "model never finishes → backstop fires; got {outcome:?}"
        );

        let offered = offered.lock().unwrap();
        assert_eq!(offered.len(), 5, "one chat call per turn");
        // Early turns (full set) still offer retrieval.
        assert!(
            offered[0]
                .iter()
                .any(|n| n == super::super::tools::VECTOR_SEMANTIC_SEARCH),
            "turn 0 offers retrieval: {:?}",
            offered[0]
        );
        // Wind-down turns (3, 4) must have dropped retrieval/read_file and kept only write/finish/abort.
        for late in [3usize, 4] {
            let set = &offered[late];
            for dropped in [
                super::super::tools::VECTOR_SEMANTIC_SEARCH,
                super::super::tools::GRAPH_FIND_SYMBOL,
                super::super::tools::GRAPH_GET_CALLERS,
                super::super::tools::READ_FILE,
                super::super::tools::REPORT_PROGRESS,
            ] {
                assert!(
                    !set.iter().any(|n| n == dropped),
                    "turn {late} must drop {dropped}: {set:?}"
                );
            }
            assert!(
                set.iter().any(|n| n == FINISH),
                "turn {late} keeps finish: {set:?}"
            );
            assert!(
                set.iter().any(|n| n == ADD_REVIEW_COMMENT),
                "turn {late} keeps add_review_comment (diff present): {set:?}"
            );
        }

        // The budget message is injected at the wind-down boundary (turn 3), so by turn 4's request the
        // conversation carries it. (Turn 0's request, before any wind-down, must not.)
        let user_text = user_text.lock().unwrap();
        assert!(
            !user_text[0].contains("Turn budget almost spent"),
            "no budget message before wind-down"
        );
        assert!(
            user_text[4].contains("Turn budget almost spent"),
            "budget message injected by the wind-down turn: {:?}",
            user_text[4]
        );
    }
}
