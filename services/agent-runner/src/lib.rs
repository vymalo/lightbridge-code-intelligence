//! Library surface of the agent runner, so integration tests (and any future in-process reuse) can
//! exercise the modules. The `agent-runner` binary (`main.rs`) is a thin orchestrator over these.
//!
//! # Module map
//!
//! Modules follow the per-task pipeline (`main.rs::run()` walks them in order):
//!
//! - [`bootstrap`] — load config ([`config`](bootstrap::config)) and talk to the control plane
//!   ([`client`](bootstrap::client), the only thing holding the runner bearer; it mints nothing).
//! - [`clone`] — checkout the repo at the head SHA using the borrowed install token.
//! - [`indexer`] — tree-sitter chunking + structural-graph extraction, with
//!   [`embeddings`](indexer::embeddings) (OpenAI-compatible vectors → control plane) feeding the
//!   semantic index.
//! - [`review`] — the native review agent loop (ADR-0026/0037): it investigates with retrieval tools
//!   and acts via mediated write tools the control plane flushes as one grouped review.
//! - [`ratelimit`] — parsing the AI gateway's rate-limit response headers (advisory budget telemetry)
//!   and the shared `Retry-After` parser the chat/embeddings clients honour on a 429.

pub mod bootstrap;
pub mod clone;
pub mod indexer;
pub mod ratelimit;
pub mod review;
