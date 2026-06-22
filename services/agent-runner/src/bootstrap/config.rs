//! Runner configuration, read from the environment the dispatcher's Job manifest injects (see
//! `control-plane/src/k8s.rs`). Only the wiring the runner needs to *find* and *authenticate to* the
//! control plane lives here; the actual task context (repo, SHAs, command) is fetched from the
//! control plane at runtime rather than trusted from env, so the env stays minimal.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;
use uuid::Uuid;

/// Where the runner looks for its JSON config file (mounted from a ConfigMap). Overridable via
/// `AGENT_CONFIG`. When the file is absent the runner falls back to legacy env vars, so a Job keeps
/// working before the chart mounts the ConfigMap.
const DEFAULT_AGENT_CONFIG_PATH: &str = "/etc/lightbridge/agent.json";

/// Default ceiling on the diff pasted into the review prompt (chars).
pub const DEFAULT_MAX_DIFF_CHARS: usize = 60_000;

/// The agent runner's file config (ADR-0021/0018). Every field is optional: a partial file overrides
/// only what it sets, and an absent file means "use env + defaults everywhere". String values support
/// `{env:VAR:-default}` (resolved by `lightbridge-config`), so secrets stay in env while models,
/// URLs, and template paths live declaratively in the ConfigMap.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub embeddings: Option<EmbeddingsFile>,
    pub review: Option<ReviewFile>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingsFile {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewFile {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Path to the reviewer's system-prompt template (a mounted file); its contents are env-subst'd.
    pub system_prompt_file: Option<String>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_diff_chars: Option<usize>,
    /// Generation params for the review model, passed through to opencode.json (the OpenAI-compatible
    /// provider). All optional — unset means the model/provider default. Numeric-string tolerant so
    /// `{env:…}`-substituted values (always strings) still deserialize.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub temperature: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub top_p: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_i64")]
    pub max_tokens: Option<i64>,
}

/// Load the agent config file if it exists. `Ok(None)` when the path is absent (use env); `Err` when
/// it exists but is malformed — a misconfiguration we want surfaced, not silently ignored.
pub fn load_file_config() -> anyhow::Result<Option<FileConfig>> {
    let path =
        std::env::var("AGENT_CONFIG").unwrap_or_else(|_| DEFAULT_AGENT_CONFIG_PATH.to_string());
    let path = Path::new(&path);
    if !path.exists() {
        return Ok(None);
    }
    lightbridge_config::load::<FileConfig>(path).map(Some)
}

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

    /// Resolve from the file config when it carries an `embeddings` block, else from env. All three
    /// fields are required either way (no default model — a misconfig fails loud).
    pub fn resolve(file: Option<&FileConfig>) -> anyhow::Result<Self> {
        match file.and_then(|f| f.embeddings.as_ref()) {
            Some(e) => Ok(Self {
                base_url: require_field("embeddings", "base_url", &e.base_url)?,
                api_key: require_field("embeddings", "api_key", &e.api_key)?,
                model: require_field("embeddings", "model", &e.model)?,
            }),
            None => Self::from_env(),
        }
    }
}

/// Validate a required config field is non-empty (after `{env:}` substitution), naming the section +
/// field in the error so a misconfigured ConfigMap is diagnosable from the first log line.
fn require_field(section: &str, name: &str, value: &str) -> anyhow::Result<String> {
    if value.trim().is_empty() {
        anyhow::bail!("config {section}.{name} is required but empty");
    }
    Ok(value.to_string())
}

/// Configuration for the OpenCode review agent's LLM — an OpenAI-compatible chat endpoint (the eaig
/// gateway in prod; ADR-0021). Like embeddings, **no default model** so a misconfigured Job fails
/// loudly. Optional as a whole: absent `LLM_MODEL`, the runner skips the review step (indexing-only).
/// Which review agent the runner drives (ADR-0026). `REVIEW_AGENT=native` opts into the in-process
/// Rust loop; anything else (incl. unset) keeps the OpenCode subprocess. Phased migration — the
/// default flips to `Native` once the loop is dogfooded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewAgent {
    #[default]
    OpenCode,
    Native,
}

impl ReviewAgent {
    fn from_env() -> Self {
        match std::env::var("REVIEW_AGENT") {
            Ok(v) if v.eq_ignore_ascii_case("native") => Self::Native,
            _ => Self::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReviewConfig {
    /// Which agent produces the review (ADR-0026); from `REVIEW_AGENT`.
    pub agent: ReviewAgent,
    /// Base URL of the OpenAI-compatible chat endpoint (the `provider.options.baseURL` opencode uses).
    pub base_url: String,
    /// API key for the gateway.
    pub api_key: String,
    /// Chat model id, referenced by opencode as `eaig/<model>`.
    pub model: String,
    /// Operator override for the reviewer's *guidance* (persona + what to focus on). From the
    /// `review.system_prompt_file` template (file config) or `REVIEW_SYSTEM_PROMPT` (env). `None` →
    /// the runner's built-in default guidance. The non-negotiable output-format contract is always
    /// appended regardless, so an override can't break parsing.
    pub system_prompt: Option<String>,
    /// Ceiling on the diff pasted into the prompt; from `review.max_diff_chars` or the default.
    pub max_diff_chars: usize,
    /// Generation params for the review model (→ opencode.json). `None` = provider/model default.
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
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
            agent: ReviewAgent::from_env(),
            base_url: require("LLM_BASE_URL")?,
            api_key: require("LLM_API_KEY")?,
            model: require("LLM_MODEL")?,
            system_prompt: std::env::var("REVIEW_SYSTEM_PROMPT")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            max_diff_chars: DEFAULT_MAX_DIFF_CHARS,
            temperature: None,
            top_p: None,
            max_tokens: None,
        }))
    }

    /// Resolve from the file config when it carries a `review` block (with a non-empty model), else
    /// from env. The system prompt comes from the `system_prompt_file` template (env-subst'd) when
    /// set. A `review` block whose model is empty disables review, same as an unset `LLM_MODEL`.
    pub fn resolve(file: Option<&FileConfig>) -> anyhow::Result<Option<Self>> {
        let Some(r) = file.and_then(|f| f.review.as_ref()) else {
            return Self::from_env();
        };
        if r.model.trim().is_empty() {
            return Ok(None); // review explicitly disabled
        }
        // Prompt source: the mounted template file when set, else the dispatcher-injected
        // `REVIEW_SYSTEM_PROMPT` env (legacy passthrough), else None (built-in default guidance).
        let system_prompt = match &r.system_prompt_file {
            Some(path) if !path.trim().is_empty() => Some(
                lightbridge_config::load_template(Path::new(path))
                    .with_context(|| format!("loading review.system_prompt_file {path}"))?,
            ),
            _ => std::env::var("REVIEW_SYSTEM_PROMPT")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        };
        Ok(Some(Self {
            agent: ReviewAgent::from_env(),
            base_url: require_field("review", "base_url", &r.base_url)?,
            api_key: require_field("review", "api_key", &r.api_key)?,
            model: require_field("review", "model", &r.model)?,
            system_prompt,
            max_diff_chars: r.max_diff_chars.unwrap_or(DEFAULT_MAX_DIFF_CHARS),
            temperature: r.temperature,
            top_p: r.top_p,
            max_tokens: r.max_tokens,
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
