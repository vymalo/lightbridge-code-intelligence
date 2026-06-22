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
//! - [`tools`] — the agent's tool surface (retrieval + control) and the in-process dispatcher.
//! - [`agent`] — the loop ([`agent::run_native_review`]), selected by `REVIEW_AGENT=native`.

pub mod agent;
pub mod chat;
pub mod tools;
