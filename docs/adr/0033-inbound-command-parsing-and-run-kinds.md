# ADR-0033: Inbound command parsing and run kinds (conversational "ask" + non-PR targets)

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

When a user @mentions the bot, the webhook detects the mention, reads the comment body, and then
**throws the body away**: it dispatches a task with a hardcoded `command_text: "review"`
(`control-plane/src/http/webhook.rs`). The agent prompt always receives `Requested review command:
review` (`agent-runner/src/review/mod.rs`) — the user's actual words never reach the model.

Observed in production ([ai-helm#436](https://github.com/ADORSYS-GIS/ai-helm/pull/436)): a comment
"*@lightbridge-assistant can you propose a better implementation?*" produced a **generic full-PR
review** instead of an answer, and then the diff-scope validator **silently dropped 4 findings**
("*4 observation(s) about code outside this PR's diff were omitted…*", `control-plane/src/review.rs`).
The user's request was ignored, work unrelated to the question ran, and results were hidden — pure
token/time waste.

[ADR-0031](0031-review-skills-commands.md) assumed this `@mention` command path worked; it does not.
And today the only supported target is a PR — there is no way to ask the bot to look at an issue, a
commit, or to simply *answer a question*.

## Decision Drivers

- **Honour the user's words:** free-text after the mention must reach the agent.
- **Right behaviour per intent:** a question deserves an answer, not a diff-scoped review; a review
  deserves diff-scoped inline findings.
- **No silent waste:** never run an out-of-scope review and then hide its output.
- **Stay focused:** within [ADR-0029](0029-focused-review-not-generic-runner.md) — review/answer about
  code, not a generic runner.

## Decision Outcome

Introduce an explicit **run kind** resolved from the inbound comment, and stop discarding the body:

1. **Parse the comment.** Strip the `@handle`, then resolve the remainder:
   - A leading **known skill/command name** ([ADR-0031](0031-review-skills-commands.md)) → that skill.
   - Otherwise **free text** → carried verbatim as the instruction.
   - Empty → default `review`.
2. **Run kinds:**
   - **`review`** (default / PR opened): diff-scoped, inline findings, validated + written back per
     [ADR-0022](0022-review-writeback-control-plane.md). Unchanged.
   - **`ask`** (free-text question, e.g. "propose a better implementation"): the agent answers using the
     same retrieval tools, and the answer is posted as a **single reply comment** on the thread —
     **not** diff-validated, **not** subject to the out-of-scope drop. This is what the ai-helm comment
     should have triggered.
   - **`skill`**: a named repo skill ([ADR-0031](0031-review-skills-commands.md)).
3. **Targets beyond PRs:** the run kind carries a `target` (PR / issue / commit / repo). `ask` and
   `review` work on a PR or an issue/ticket; the output sink follows the target (PR review vs issue
   comment). The diff-scope rules apply only when a diff exists.
4. **No silent drops in `ask`.** For `review`, out-of-scope observations are **surfaced in a collapsible
   "outside this diff" section** rather than dropped (or posted to the run record), so the work isn't
   thrown away — the count line stays but the content is recoverable.
5. **The agent knows its name and the request.** The prompt states the bot handle and includes the
   parsed instruction + run kind, so "can you propose…" is acted on directly.

`command_text` is replaced/augmented by a structured `{ kind, target, instruction, skill? }` payload
threaded webhook → task → runner (the plumbing to `COMMAND`/prompt already exists; it's just unused).

### Consequences

- Good: fixes the ai-helm incident class — the user's request reaches the agent and gets the right
  behaviour; no hidden output; the bot can be pointed at issues/tickets, not just PRs.
- Good: makes [ADR-0031](0031-review-skills-commands.md) actually reachable (it depends on this path).
- Bad: a real comment grammar + run-kind resolver to design and test; more task payload surface;
  `ask` is a new output sink (reply comment) with its own formatting.
- Neutral: `review` semantics are unchanged; this adds intents alongside it.

## Pros and Cons of the Options

### Structured run kind + parse the body (chosen)
- Good: intent-correct behaviour, no waste, unlocks skills + non-PR targets.
- Bad: grammar/UX + resolver to build.

### Keep hardcoded `review`, just stop dropping out-of-scope
- Good: tiny change.
- Bad: still ignores the user's actual request — the headline bug remains.

## More Information

- Root cause: `control-plane/src/http/webhook.rs` (`handle_issue_comment`, hardcoded `command_text`),
  `agent-runner/src/review/mod.rs` (`build_prompt`), drop logic in `control-plane/src/review.rs`.
- Depends on [ADR-0026](0026-native-review-agent.md) (the loop/control tools) and feeds
  [ADR-0031](0031-review-skills-commands.md). Bot handle from `GITHUB_APP_HANDLE`.
- Incident: [ai-helm#436 review](https://github.com/ADORSYS-GIS/ai-helm/pull/436#pullrequestreview-4543004829),
  trigger [comment](https://github.com/ADORSYS-GIS/ai-helm/pull/436#issuecomment-4767080893).
