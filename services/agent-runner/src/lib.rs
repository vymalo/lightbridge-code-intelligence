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
//! - [`review`] — drive the review agent and parse its findings.
//!
//! The two `bin/` targets (`vector-mcp`, `graph-mcp`) are stdio MCP servers that wrap the
//! control-plane retrieval API for the review agent; they reuse [`bootstrap::client`].

pub mod bootstrap;
pub mod clone;
pub mod indexer;
pub mod review;
