//! Native Rust review agent (ADR-0026) — the in-process replacement for the OpenCode subprocess.
//!
//! Instead of spawning OpenCode and scraping a ` ```json ` block from its stdout, the runner drives
//! its own agent loop against the eaig OpenAI-compatible **Chat Completions** endpoint with function
//! calling. The review is returned by the model **calling a tool** (`submit_findings`), so the result
//! is validated at a tool boundary rather than parsed from free text.
//!
//! Built in phases behind `REVIEW_AGENT=native|opencode` (ADR-0026). This module currently holds:
//!
//! - [`chat`] — the Chat Completions client (request/response types + tool-call protocol).
//!
//! Still to come (later PRs): the tool registry + in-process dispatch (the same retrieval tools the
//! MCP servers expose, plus the control tools `submit_findings`/`report_progress`/`abort`), and the
//! loop itself.

pub mod chat;
