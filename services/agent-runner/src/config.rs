//! Runner configuration, read from the environment the dispatcher's Job manifest injects (see
//! `control-plane/src/k8s.rs`). Only the wiring the runner needs to *find* and *authenticate to* the
//! control plane lives here; the actual task context (repo, SHAs, command) is fetched from the
//! control plane at runtime rather than trusted from env, so the env stays minimal.

use uuid::Uuid;

/// Everything the runner needs to start: which task it is, and how to reach the control plane.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub task_id: Uuid,
    pub control_plane_url: String,
    pub runner_token: String,
    /// Directory the repository is cloned into. Defaults to `/workspace` (an emptyDir in the Job).
    pub workdir: String,
}

impl RunnerConfig {
    /// Parse from process env. Errors name the missing/invalid variable so a misconfigured Job is
    /// diagnosable from the runner's first log line.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            task_id: parse_required("TASK_ID")?,
            control_plane_url: require("CONTROL_PLANE_URL")?,
            runner_token: require("AGENT_RUNNER_TOKEN")?,
            workdir: std::env::var("WORKDIR").unwrap_or_else(|_| "/workspace".to_string()),
        })
    }
}

/// Configuration for the OpenAI-compatible embeddings API (ADR-0018). All three fields are
/// required — no default model, so a misconfigured Job fails loudly with a named variable.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Base URL of the OpenAI-compatible endpoint (no trailing `/v1`).
    /// Prod: `https://core-gateway-internal.envoy-gateway-system.svc.cluster.local`
    pub base_url: String,
    /// API key presented as `Authorization: Bearer`. Prod key: `converse_openai_api_key`.
    pub api_key: String,
    /// Model identifier, e.g. `text-embedding-3-small`. The schema expects 1536-dim vectors
    /// matching that model; choosing a different-dimension model requires a migration (ADR-0018).
    pub model: String,
}

impl EmbeddingsConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            base_url: require("EMBEDDINGS_BASE_URL")?,
            api_key: require("EMBEDDINGS_API_KEY")?,
            model: require("EMBEDDINGS_MODEL")?,
        })
    }
}

/// Configuration for the OpenCode review agent's LLM — an OpenAI-compatible chat endpoint (the eaig
/// gateway in prod; ADR-0021). Like embeddings, **no default model** so a misconfigured Job fails
/// loudly. Optional as a whole: absent `LLM_MODEL`, the runner skips the review step (indexing-only).
#[derive(Debug, Clone)]
pub struct ReviewConfig {
    /// Base URL of the OpenAI-compatible chat endpoint (the `provider.options.baseURL` opencode uses).
    pub base_url: String,
    /// API key for the gateway.
    pub api_key: String,
    /// Chat model id, referenced by opencode as `eaig/<model>`.
    pub model: String,
    /// Operator override for the reviewer's *guidance* (persona + what to focus on), from
    /// `REVIEW_SYSTEM_PROMPT`. `None` → the runner's built-in default guidance. The non-negotiable
    /// output-format contract is always appended regardless, so an override can't break parsing.
    pub system_prompt: Option<String>,
}

impl ReviewConfig {
    /// Returns `None` when `LLM_MODEL` is unset (review disabled), `Err` when it's set but the
    /// base URL / key are missing (a misconfiguration we want to surface, not silently skip).
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        if std::env::var("LLM_MODEL")
            .ok()
            .filter(|v| !v.is_empty())
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(Self {
            base_url: require("LLM_BASE_URL")?,
            api_key: require("LLM_API_KEY")?,
            model: require("LLM_MODEL")?,
            system_prompt: std::env::var("REVIEW_SYSTEM_PROMPT")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }))
    }
}

fn require(name: &str) -> anyhow::Result<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => anyhow::bail!("{name} is required but unset/empty"),
    }
}

fn parse_required(name: &str) -> anyhow::Result<Uuid> {
    let raw = require(name)?;
    Uuid::parse_str(&raw).map_err(|_| anyhow::anyhow!("{name} is not a valid UUID: {raw:?}"))
}
