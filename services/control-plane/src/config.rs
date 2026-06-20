//! Control-plane file configuration (RFC-0001 + ADR-0021).
//!
//! Like the agent runner, the control plane reads an optional JSON config file (mounted from a Helm
//! ConfigMap) with `{env:VAR:-default}` substitution, instead of a sprawl of env vars. **File when
//! present, else env** — an absent file means today's env vars + defaults still apply, so prod keeps
//! running until the ConfigMap is mounted.
//!
//! Scope: the agent-Job knobs the dispatcher stamps into each Job (namespace, image, deadline, the
//! agent ConfigMap to mount, …) and the dispatcher loop timings. Each field is optional and falls
//! back to its prior env/default individually.

use std::path::Path;

use serde::Deserialize;

/// Where the control plane looks for its config file; overridable via `CONTROL_PLANE_CONFIG`.
const DEFAULT_CONFIG_PATH: &str = "/etc/lightbridge/control-plane.json";

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub agent: AgentSection,
    pub dispatcher: DispatcherSection,
    pub review: ReviewSection,
}

/// Review-feedback knobs the control plane applies to the PR: lifecycle reactions and outcome labels.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewSection {
    /// React to the PR description across the review lifecycle (👀 started → 🎉 done / 😕 errored).
    /// Defaults to enabled when unset.
    pub reactions: Option<bool>,
    /// Label added whenever a review is posted (e.g. `lightbridge-reviewed`). `None` → not added.
    pub label_reviewed: Option<String>,
    /// Label added when the review has ≥1 finding of any severity (e.g. `needs-review`).
    pub label_findings: Option<String>,
    /// Label added when the review has ≥1 `error`-severity finding (e.g. `bug`).
    pub label_error: Option<String>,
}

impl ReviewSection {
    /// Reactions are on unless explicitly disabled.
    pub fn reactions_enabled(&self) -> bool {
        self.reactions.unwrap_or(true)
    }
}

/// Knobs for the per-task agent Job the dispatcher launches (mirrors `KubeLauncher`).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentSection {
    pub namespace: Option<String>,
    pub runner_image: Option<String>,
    pub service_account: Option<String>,
    pub control_plane_url: Option<String>,
    /// Secret holding the internal CA (`ca.crt`) to mount into the Job.
    pub ca_secret: Option<String>,
    /// ConfigMap (in the agents namespace) holding the runner's `agent.json` + prompt templates, to
    /// mount at `/etc/lightbridge` so the runner reads its file config. `None` → not mounted.
    pub config_configmap: Option<String>,
    /// The Job's `activeDeadlineSeconds` runtime cap.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_i64")]
    pub job_deadline_seconds: Option<i64>,
    /// Legacy passthrough: inline reviewer prompt injected as `REVIEW_SYSTEM_PROMPT`. Prefer mounting
    /// the template via `config_configmap` instead.
    pub review_system_prompt: Option<String>,
    /// The runner container's k8s `resources` block (requests/limits), passed through verbatim into
    /// the Job's container spec. A raw object so operators can express any valid shape (and use
    /// `{env:…}` inside). `None` → no resources set (cluster defaults / LimitRange apply).
    pub resources: Option<serde_json::Value>,
}

/// Dispatcher loop timings (seconds). Each falls back to its built-in default in `dispatcher.rs`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DispatcherSection {
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub claim_lease_seconds: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub poll_fallback_seconds: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub launch_backoff_seconds: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub reap_interval_seconds: Option<u64>,
}

/// Load the control-plane config file if it exists. `Ok(None)` when absent (use env); `Err` when it
/// exists but is malformed.
pub fn load_file_config() -> anyhow::Result<Option<FileConfig>> {
    let path =
        std::env::var("CONTROL_PLANE_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
    let path = Path::new(&path);
    if !path.exists() {
        return Ok(None);
    }
    lightbridge_config::load::<FileConfig>(path).map(Some)
}
