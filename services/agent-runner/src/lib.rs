//! Library surface of the agent runner, so integration tests (and any future in-process reuse) can
//! exercise the modules. The `agent-runner` binary (`main.rs`) is a thin orchestrator over these.

pub mod client;
pub mod clone;
pub mod config;
pub mod embeddings;
pub mod indexer;
pub mod review;
