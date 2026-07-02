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

/// Default per-request timeout for the chat HTTP client (seconds). Deliberately **generous**: eaig can
/// legitimately take up to ~2 minutes to answer a turn, so an aggressive timeout would kill a
/// slow-but-valid response (ADR-0039). Overridable via `LLM_REQUEST_TIMEOUT_SECS` /
/// `review.request_timeout_secs`.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 180;

/// Default number of **retries** (so total attempts = retries + 1) on a transient turn failure
/// (connect/timeout, HTTP 429, HTTP 5xx). 4xx other than 429 is deterministic and never retried.
/// Overridable via `LLM_MAX_RETRIES` / `review.max_retries`.
pub const DEFAULT_MAX_RETRIES: u32 = 2;

/// Default per-run circuit-breaker threshold: after this many *consecutive* turn failures the run
/// fails fast rather than burning the whole turn budget (ADR-0039). Overridable via
/// `LLM_CIRCUIT_BREAKER_THRESHOLD` / `review.circuit_breaker_threshold`.
pub const DEFAULT_CIRCUIT_BREAKER_THRESHOLD: u32 = 3;

/// Default ceiling on model turns before the run is cut off (each turn is one chat round-trip). On the
/// deepseek model a turn is ~6s and the agent records roughly one finding per turn, so the old ceiling
/// of 16 was far too tight for a real PR — a run could exhaust its budget with findings still buffered.
/// Operator-tunable via `review.max_turns` / `LLM_MAX_TURNS`. Overridable but defaults generously.
pub const DEFAULT_MAX_TURNS: usize = 40;

/// Default ceiling on how many read-only tool calls (search / graph / `read_file`) the runner executes
/// **concurrently** within one turn — the batch size for risk-first review (ADR-0042). The model can
/// emit many tool calls in a turn; we run up to this many in parallel so a batch of reads costs one
/// round-trip's latency instead of N. Read-only only: write/finish/abort keep their ordered, sequential
/// handling. Operator-tunable via `review.max_batch_size` / `LLM_MAX_BATCH_SIZE`.
pub const DEFAULT_MAX_BATCH_SIZE: usize = 8;

/// Default cumulative read budgets for risk-first review (ADR-0042): once a budget is spent the runner
/// drops the matching tool from the offered set and nudges the model to converge — so a review reads
/// *enough* to be confident, then stops, instead of grinding through the whole repo. `max_files_read`
/// caps `read_file` calls, `max_searches` caps retrieval (vector + graph) calls, and `max_batches` caps
/// investigation rounds (turns that issue ≥1 read-only call) before forcing the wind-down. Generous
/// defaults; operator-tunable via `review.*` / `LLM_MAX_*`.
pub const DEFAULT_MAX_FILES_READ: usize = 30;
pub const DEFAULT_MAX_SEARCHES: usize = 15;
pub const DEFAULT_MAX_BATCHES: usize = 6;

/// SAST (opengrep) defaults (ADR-0061). The whole feature is **opt-in** (default disabled) so the
/// rollout is image-then-config: an existing deploy without the opengrep-bearing image is unaffected.
pub const DEFAULT_SAST_BIN: &str = "opengrep";
/// Vendored, pinned ruleset baked into the runner image (see the Dockerfile). A local dir, so scans are
/// hermetic — no registry fetch at runtime. Operator-overridable to narrow the rule set to their stack.
pub const DEFAULT_SAST_RULES: &str = "/opt/opengrep-rules";
/// Minimum SARIF level to surface. `error` (high-signal, mostly real security/correctness) by default so
/// the first rollout doesn't flood PRs; lower to `warning`/`note` to widen.
pub const DEFAULT_SAST_MIN_SEVERITY: &str = "error";
/// Cap on findings posted per review, so a pathological file can't bury the PR. Excess is logged, not
/// silently dropped (ADR-0033).
pub const DEFAULT_SAST_MAX_FINDINGS: usize = 25;
/// Wall-clock ceiling on one opengrep scan; on timeout the pass is abandoned (non-fatal).
pub const DEFAULT_SAST_TIMEOUT_SECS: u64 = 300;

/// The agent runner's file config (ADR-0021/0018). Every field is optional: a partial file overrides
/// only what it sets, and an absent file means "use env + defaults everywhere". String values support
/// `{env:VAR:-default}` (resolved by `lightbridge-config`), so secrets stay in env while models,
/// URLs, and template paths live declaratively in the ConfigMap.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub embeddings: Option<EmbeddingsFile>,
    pub review: Option<ReviewFile>,
    /// Deterministic SAST pass (ADR-0061). Absent or `enabled: false` ⇒ no SAST.
    pub sast: Option<SastFile>,
}

/// File config for the deterministic SAST pass (ADR-0061). Every field is optional; an absent block (or
/// `enabled: false`) disables SAST entirely. Bool/numeric-string tolerant so `{env:…}`-substituted
/// values still deserialize.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SastFile {
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_bool")]
    pub enabled: Option<bool>,
    /// opengrep binary name/path; defaults to `opengrep` on PATH.
    pub bin: Option<String>,
    /// `--config` value: a local rules dir (default: the vendored set) or a registry ruleset.
    pub rules: Option<String>,
    /// Minimum SARIF level to surface (`error`|`warning`|`note`).
    pub min_severity: Option<String>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_findings: Option<usize>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingsFile {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Per-model tuning block (ADR-0051). Embeddings have few knobs today; `config` keeps the shape
    /// uniform with the review models and is where future ones land. Unset = defaults.
    #[serde(default)]
    pub config: Option<EmbeddingsTuningFile>,
}

/// Per-model tuning for the embeddings model (ADR-0051). Numeric-string tolerant for `{env:}` values.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingsTuningFile {
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub request_timeout_secs: Option<u64>,
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
    /// Generation params for the review model. All optional — unset means the model/provider default.
    /// Numeric-string tolerant so `{env:…}`-substituted values (always strings) still deserialize.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub temperature: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub top_p: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_i64")]
    pub max_tokens: Option<i64>,
    /// Provider-specific passthrough generation params, merged verbatim into the chat request body —
    /// for knobs the typed fields don't cover, notably a **reasoning budget** (e.g. `thinking`,
    /// `reasoning_effort`) to stop a reasoning model over-thinking. A JSON object; `None` = nothing
    /// extra. The operator owns correctness; unknown fields the gateway/model ignores are harmless.
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    /// Resilience knobs (ADR-0039). All optional — unset falls back to the safe defaults above so a
    /// deploy works without an ai-helm values change. Numeric-string tolerant for `{env:…}` values.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub request_timeout_secs: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub max_retries: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub circuit_breaker_threshold: Option<u64>,
    /// Ceiling on model turns before the run is cut off (operator-tunable). Unset = [`DEFAULT_MAX_TURNS`].
    /// Numeric-string tolerant for `{env:…}`-substituted values.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_turns: Option<usize>,
    /// Max read-only tool calls run concurrently per turn (ADR-0042). Unset = [`DEFAULT_MAX_BATCH_SIZE`].
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_batch_size: Option<usize>,
    /// Cumulative read budgets (ADR-0042). Unset = the `DEFAULT_MAX_*` constants.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_files_read: Option<usize>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_searches: Option<usize>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub max_batches: Option<usize>,
    /// Model context window in tokens (ADR-0045). When set, the agent budgets its conversation against
    /// it — winding down before overflow and trimming old tool output — instead of failing a 400 when
    /// the history grows too large. Unset = no budgeting (unchanged behaviour).
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_usize")]
    pub context_window: Option<usize>,
    /// Stream the chat response (SSE) and reassemble it client-side instead of awaiting the whole
    /// completion (ADR-0039 / #206). `Some(true)` enables it; unset falls back to the `LLM_STREAM` env
    /// (legacy/local toggle), else off. Streaming bounds a long-but-progressing turn by a per-chunk idle
    /// timeout rather than one whole-request timeout — useful for a heavy-reasoning model (e.g. GLM).
    /// Bool-tolerant like the numeric knobs above, so a `{env:…}`-substituted string (e.g.
    /// `"{env:LLM_STREAM:-true}"`) still deserializes instead of failing the config.
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_bool")]
    pub stream: Option<bool>,
    /// Two-tier review (ADR-0062): a fully-independent config for the FAST tier (automatic
    /// `pull_request opened`). When present it is a COMPLETE review block (its own model, gateway, prompt,
    /// reasoning budget, timeout, …) — NOT an overlay on the flat fields. When absent, the FAST tier
    /// falls back to the flat `review.*` block (back-compat: an older values file with no tier blocks).
    /// A nested block's own `fast`/`deep` are ignored.
    #[serde(default)]
    pub fast: Option<Box<ReviewFile>>,
    /// Two-tier review (ADR-0062): a fully-independent config for the DEEP tier (`@mention`). Same shape
    /// and fallback as `fast`. This is where the strong model + 2h timeout live; the FAST block carries
    /// the cheap model + short timeout.
    #[serde(default)]
    pub deep: Option<Box<ReviewFile>>,
    /// Per-tier tool allowlist (ADR-0062): the exact set of tools this tier offers the model, e.g.
    /// `["add_review_comment", "finish", "abort"]` for a diff-only FAST pass with no retrieval. A closed
    /// [`ReviewTool`] enum, so an unknown name **fails at deserialize** (serde names the valid variants)
    /// rather than being a free-form string we'd have to hand-check. When unset, the tier uses the
    /// built-in default (the full surface for DEEP; the wind-down write/finish/abort set for FAST).
    /// Externalizing it lets an operator tune each tier's surface from the ConfigMap.
    #[serde(default)]
    pub tools: Option<Vec<ReviewTool>>,
}

/// A tool the review agent can be configured to offer (ADR-0062). A **closed enum** so a per-tier
/// `review.<tier>.tools` allowlist is validated when the config is parsed — an unknown name fails the
/// config with serde listing the valid variants — instead of a free-form string the runner has to
/// re-validate by hand. Each serde name is the EXACT tool name the agent dispatches (see
/// [`crate::review::native::tools`]); a sync test asserts the enum can't drift from that surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ReviewTool {
    #[serde(rename = "lightbridge_vector_semantic_search")]
    VectorSemanticSearch,
    #[serde(rename = "lightbridge_graph_find_symbol")]
    GraphFindSymbol,
    #[serde(rename = "lightbridge_graph_get_callers")]
    GraphGetCallers,
    #[serde(rename = "read_file")]
    ReadFile,
    #[serde(rename = "add_review_comment")]
    AddReviewComment,
    #[serde(rename = "retract_finding")]
    RetractFinding,
    #[serde(rename = "add_comment")]
    AddComment,
    #[serde(rename = "finish")]
    Finish,
    #[serde(rename = "report_progress")]
    ReportProgress,
    #[serde(rename = "abort")]
    Abort,
    /// External-knowledge MCP tools (ADR-0066), mediated by the control plane: whatever the
    /// configured MCP servers (e.g. brave-search, context7) currently expose, discovered
    /// dynamically at run start — never a hardcoded per-provider tool. A single sentinel rather
    /// than one variant per downstream tool, since the actual set isn't known at compile time.
    /// Available to any tier; unlike the rest of this enum, it's not a single dispatchable tool but
    /// a whole discoverable set gated the same way as everything else — via this allowlist.
    #[serde(rename = "mcp_tools")]
    McpTools,
}

impl ReviewTool {
    /// Every variant, in the canonical tool order — the operator-facing list of valid `tools` values.
    pub const ALL: [ReviewTool; 11] = [
        ReviewTool::VectorSemanticSearch,
        ReviewTool::GraphFindSymbol,
        ReviewTool::GraphGetCallers,
        ReviewTool::ReadFile,
        ReviewTool::AddReviewComment,
        ReviewTool::RetractFinding,
        ReviewTool::AddComment,
        ReviewTool::Finish,
        ReviewTool::ReportProgress,
        ReviewTool::Abort,
        ReviewTool::McpTools,
    ];

    /// The canonical tool name the agent dispatches — the exact string in [`crate::review::native::tools`].
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewTool::VectorSemanticSearch => "lightbridge_vector_semantic_search",
            ReviewTool::GraphFindSymbol => "lightbridge_graph_find_symbol",
            ReviewTool::GraphGetCallers => "lightbridge_graph_get_callers",
            ReviewTool::ReadFile => "read_file",
            ReviewTool::AddReviewComment => "add_review_comment",
            ReviewTool::RetractFinding => "retract_finding",
            ReviewTool::AddComment => "add_comment",
            ReviewTool::Finish => "finish",
            ReviewTool::ReportProgress => "report_progress",
            ReviewTool::Abort => "abort",
            ReviewTool::McpTools => "mcp_tools",
        }
    }
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
    /// Per-request timeout (seconds) for one embeddings call (ADR-0051). From `embeddings.config
    /// .request_timeout_secs` / `EMBEDDINGS_REQUEST_TIMEOUT_SECS`, else [`DEFAULT_REQUEST_TIMEOUT_SECS`].
    pub request_timeout_secs: u64,
}

impl EmbeddingsConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            base_url: require("EMBEDDINGS_BASE_URL")?,
            api_key: require("EMBEDDINGS_API_KEY")?,
            model: require("EMBEDDINGS_MODEL")?,
            request_timeout_secs: parse_env_u64("EMBEDDINGS_REQUEST_TIMEOUT_SECS")
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
        })
    }

    /// Resolve from the file config when it carries an `embeddings` block, else from env. The three
    /// connection fields are required either way (no default model — a misconfig fails loud); the
    /// `config` block is optional.
    pub fn resolve(file: Option<&FileConfig>) -> anyhow::Result<Self> {
        match file.and_then(|f| f.embeddings.as_ref()) {
            Some(e) => Ok(Self {
                base_url: require_field("embeddings", "base_url", &e.base_url)?,
                api_key: require_field("embeddings", "api_key", &e.api_key)?,
                model: require_field("embeddings", "model", &e.model)?,
                request_timeout_secs: e
                    .config
                    .as_ref()
                    .and_then(|c| c.request_timeout_secs)
                    .or_else(|| parse_env_u64("EMBEDDINGS_REQUEST_TIMEOUT_SECS"))
                    .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
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

/// Configuration for the native review agent's LLM — an OpenAI-compatible Chat Completions endpoint
/// (the eaig gateway in prod; ADR-0018/0026). Like embeddings, **no default model** so a misconfigured
/// Job fails loudly. Optional as a whole: absent `LLM_MODEL`, the runner skips the review step
/// (indexing-only).
#[derive(Debug, Clone)]
pub struct ReviewConfig {
    /// Base URL of the OpenAI-compatible Chat Completions endpoint.
    pub base_url: String,
    /// API key for the gateway.
    pub api_key: String,
    /// Chat model id.
    pub model: String,
    /// The reviewer's *guidance* (persona + what to focus on), from the `review.system_prompt_file`
    /// template (file config) or `REVIEW_SYSTEM_PROMPT` (env). **Required — there is no built-in
    /// default** (ADR-0037): a prompt this central is operational config, owned in the ai-helm chart
    /// (`config.reviewSystemPrompt`), not a stale code constant. If review is enabled but no prompt is
    /// configured, [`resolve`]/[`from_env`] error (fail closed) rather than running a weak fallback.
    pub system_prompt: String,
    /// Ceiling on the diff pasted into the prompt; from `review.max_diff_chars` or the default.
    pub max_diff_chars: usize,
    /// Ceiling on model turns before the run is cut off; from `review.max_turns` (or `LLM_MAX_TURNS`
    /// env) or [`DEFAULT_MAX_TURNS`]. Operator-tunable so a large PR isn't truncated by a tight budget.
    pub max_turns: usize,
    /// Max read-only tool calls executed concurrently within one turn (ADR-0042); from
    /// `review.max_batch_size` (or `LLM_MAX_BATCH_SIZE`) or [`DEFAULT_MAX_BATCH_SIZE`]. Clamped to ≥1.
    pub max_batch_size: usize,
    /// Cumulative read budgets (ADR-0042): once spent, the matching tool is dropped and the model is
    /// nudged to converge. From `review.max_*` (or `LLM_MAX_*`) or the `DEFAULT_MAX_*` constants.
    pub max_files_read: usize,
    pub max_searches: usize,
    pub max_batches: usize,
    /// Model context window in tokens (ADR-0045). `Some(n)` enables conversation budgeting: the loop
    /// winds down + trims old tool output as the estimate nears `n`, and finalizes (never discards
    /// findings) on an overflow error. `None` = no budgeting. From `review.context_window` /
    /// `LLM_CONTEXT_WINDOW`; a 0 is treated as unset.
    pub context_window: Option<usize>,
    /// Generation params for the review model. `None` = provider/model default.
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    /// Provider-specific passthrough request fields (reasoning budget etc.), from `review.extra`,
    /// merged verbatim into the chat body. Empty = nothing extra.
    pub extra: serde_json::Map<String, serde_json::Value>,
    /// Stream the chat response (SSE) and reassemble client-side (ADR-0039 / #206). From
    /// `review.stream`, else the `LLM_STREAM` env, else false.
    pub stream: bool,
    /// Resilience policy for the LLM transport: timeout, retry/backoff, circuit breaker (ADR-0039).
    /// Always present (defaults applied at resolve time). These are the **primary** model's per-request
    /// knobs + the loop-level breaker.
    pub resilience: ResilienceConfig,
    /// FAST tier (ADR-0062): when `true` the agent runs a single diff-only turn with **no retrieval
    /// tools** and no investigation loop (SAST still posts independently). This is a **per-task** flag —
    /// NOT from file config — set by `main.rs` from the task context's `tier` (`fast` → `true`); a Job
    /// runs one task, so mutating it per-run on the resolved config is sound. Defaults to `false` (deep).
    pub fast: bool,
    /// Per-tier tool allowlist (ADR-0062): when `Some`, the authoritative set of tools this tier offers
    /// the model (a non-empty list of [`ReviewTool`]). `None` = the built-in default for the tier. From
    /// `review.<tier>.tools`.
    pub tools: Option<Vec<ReviewTool>>,
}

/// Resilience policy for the review LLM transport (ADR-0039). eaig can legitimately take ~2 minutes
/// per turn, so the timeout is deliberately generous; retries are bounded and only fire on transient
/// failures; and a per-run circuit breaker fails fast before the turn budget is exhausted.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    /// Per-request timeout (seconds) for one chat round-trip.
    pub request_timeout_secs: u64,
    /// Retries on a transient turn failure (total attempts = `max_retries + 1`).
    pub max_retries: u32,
    /// Consecutive turn-failures before the per-run circuit breaker trips and the run fails fast.
    pub circuit_breaker_threshold: u32,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            max_retries: DEFAULT_MAX_RETRIES,
            circuit_breaker_threshold: DEFAULT_CIRCUIT_BREAKER_THRESHOLD,
        }
    }
}

impl ResilienceConfig {
    /// From env vars, each falling back to the safe default when unset/unparseable. Used by the
    /// env-config path; the file-config path builds this from the `review.*` fields.
    fn from_env() -> Self {
        Self {
            request_timeout_secs: parse_env_u64("LLM_REQUEST_TIMEOUT_SECS")
                .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
            max_retries: parse_env_u64("LLM_MAX_RETRIES")
                .map(|n| n as u32)
                .unwrap_or(DEFAULT_MAX_RETRIES),
            circuit_breaker_threshold: parse_env_u64("LLM_CIRCUIT_BREAKER_THRESHOLD")
                .map(|n| n as u32)
                .unwrap_or(DEFAULT_CIRCUIT_BREAKER_THRESHOLD),
        }
    }
}

/// Whether streaming is enabled via the legacy/local `LLM_STREAM` env toggle (`"1"` = on). The
/// file-config `review.stream` takes precedence over this; it exists so a local run or a stale
/// deploy can still opt in without an `agent.json` change.
fn stream_from_env() -> bool {
    std::env::var("LLM_STREAM").ok().as_deref() == Some("1")
}

/// Parse a `u64` env var, returning `None` when unset, empty, or unparseable (the caller applies its
/// own default). A bad value is logged so a fat-fingered env is diagnosable rather than silently ignored.
fn parse_env_u64(name: &str) -> Option<u64> {
    match std::env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => {
                tracing::warn!(var = name, value = %raw, "ignoring non-numeric env value; using default");
                None
            }
        },
        _ => None,
    }
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
            system_prompt: require_system_prompt(None)?,
            max_diff_chars: DEFAULT_MAX_DIFF_CHARS,
            max_turns: parse_env_u64("LLM_MAX_TURNS")
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_TURNS)
                .max(1),
            max_batch_size: parse_env_u64("LLM_MAX_BATCH_SIZE")
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_BATCH_SIZE)
                .max(1),
            max_files_read: parse_env_u64("LLM_MAX_FILES_READ")
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_FILES_READ)
                .max(1),
            max_searches: parse_env_u64("LLM_MAX_SEARCHES")
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_SEARCHES)
                .max(1),
            max_batches: parse_env_u64("LLM_MAX_BATCHES")
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_BATCHES)
                .max(1),
            context_window: parse_env_u64("LLM_CONTEXT_WINDOW")
                .map(|n| n as usize)
                .filter(|&n| n > 0),
            temperature: None,
            top_p: None,
            max_tokens: None,
            // No env var for arbitrary passthrough params — the file path (`review.extra`) is where a
            // reasoning budget is set.
            extra: serde_json::Map::new(),
            stream: stream_from_env(),
            resilience: ResilienceConfig::from_env(),
            fast: false, // per-task; main.rs sets it from the task tier (ADR-0062)
            // No env knob for a per-tier tool allowlist — the file path (`review.<tier>.tools`) is where
            // an operator declares it; env config uses the built-in per-tier defaults.
            tools: None,
        }))
    }

    /// Resolve from the file config when it carries a `review` block (with a non-empty model), else
    /// from env. The system prompt comes from the `system_prompt_file` template (env-subst'd) when
    /// set. A `review` block whose model is empty disables review, same as an unset `LLM_MODEL`.
    pub fn resolve(file: Option<&FileConfig>) -> anyhow::Result<Option<Self>> {
        let Some(r) = file.and_then(|f| f.review.as_ref()) else {
            return Self::from_env();
        };
        Self::from_review_file(r)
    }

    /// Resolve ONE review block — the flat `review.*`, or a per-tier `review.fast`/`review.deep` block,
    /// each a *complete* config (ADR-0062) — into a [`ReviewConfig`]. `Ok(None)` when the model is empty
    /// (review disabled). Any nested `fast`/`deep` on `r` is ignored (tiers don't nest).
    fn from_review_file(r: &ReviewFile) -> anyhow::Result<Option<Self>> {
        if r.model.trim().is_empty() {
            return Ok(None); // review explicitly disabled
        }
        // Primary effective values (file wins, else env, else default).
        let context_window = r
            .context_window
            .or_else(|| parse_env_u64("LLM_CONTEXT_WINDOW").map(|n| n as usize))
            .filter(|&n| n > 0);
        let temperature = r.temperature;
        let top_p = r.top_p;
        let max_tokens = r.max_tokens;
        // `review.extra`: a free-form object of provider-specific params (reasoning budget etc.). Only
        // a JSON object is meaningful as request fields; a non-object is a misconfiguration — warn and
        // ignore it rather than silently dropping it (review still runs; the tuning just doesn't apply).
        let extra = match r.extra.as_ref() {
            Some(serde_json::Value::Object(o)) => o.clone(),
            None => serde_json::Map::new(),
            Some(_) => {
                tracing::warn!(
                    "review.extra is not a JSON object; ignoring it (expected a map of request fields)"
                );
                serde_json::Map::new()
            }
        };
        let request_timeout_secs = r
            .request_timeout_secs
            .or_else(|| parse_env_u64("LLM_REQUEST_TIMEOUT_SECS"))
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS);
        // Per-tier tool allowlist (ADR-0062): names are already validated at deserialize (the closed
        // `ReviewTool` enum). Only an EMPTY list needs rejecting here — a tier with no tools can't act.
        let tools = match &r.tools {
            Some(t) if t.is_empty() => anyhow::bail!(
                "review.tools is set but empty — a tier with no tools can't act. Remove the key (use \
                 the built-in default) or list at least one of: {}",
                ReviewTool::ALL
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(t) => Some(t.clone()),
            None => None,
        };
        let max_retries = r
            .max_retries
            .or_else(|| parse_env_u64("LLM_MAX_RETRIES"))
            .map(|n| n as u32)
            .unwrap_or(DEFAULT_MAX_RETRIES);
        Ok(Some(Self {
            base_url: require_field("review", "base_url", &r.base_url)?,
            api_key: require_field("review", "api_key", &r.api_key)?,
            model: require_field("review", "model", &r.model)?,
            system_prompt: require_system_prompt(r.system_prompt_file.as_deref())?,
            max_diff_chars: r.max_diff_chars.unwrap_or(DEFAULT_MAX_DIFF_CHARS),
            // File config wins when set, else env (an operator can still tune via env with a
            // ConfigMap mounted), else the generous default.
            max_turns: r
                .max_turns
                .or_else(|| parse_env_u64("LLM_MAX_TURNS").map(|n| n as usize))
                .unwrap_or(DEFAULT_MAX_TURNS)
                .max(1),
            max_batch_size: r
                .max_batch_size
                .or_else(|| parse_env_u64("LLM_MAX_BATCH_SIZE").map(|n| n as usize))
                .unwrap_or(DEFAULT_MAX_BATCH_SIZE)
                .max(1),
            max_files_read: r
                .max_files_read
                .or_else(|| parse_env_u64("LLM_MAX_FILES_READ").map(|n| n as usize))
                .unwrap_or(DEFAULT_MAX_FILES_READ)
                .max(1),
            max_searches: r
                .max_searches
                .or_else(|| parse_env_u64("LLM_MAX_SEARCHES").map(|n| n as usize))
                .unwrap_or(DEFAULT_MAX_SEARCHES)
                .max(1),
            max_batches: r
                .max_batches
                .or_else(|| parse_env_u64("LLM_MAX_BATCHES").map(|n| n as usize))
                .unwrap_or(DEFAULT_MAX_BATCHES)
                .max(1),
            context_window,
            temperature,
            top_p,
            max_tokens,
            extra,
            // File wins; else the `LLM_STREAM` env (legacy/local), else off.
            stream: r.stream.unwrap_or_else(stream_from_env),
            resilience: ResilienceConfig {
                request_timeout_secs,
                max_retries,
                circuit_breaker_threshold: r
                    .circuit_breaker_threshold
                    .or_else(|| parse_env_u64("LLM_CIRCUIT_BREAKER_THRESHOLD"))
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_CIRCUIT_BREAKER_THRESHOLD),
            },
            fast: false, // set by resolve_tiers / main per the task tier (ADR-0062)
            tools,
        }))
    }

    /// Resolve BOTH review tiers (ADR-0062). Each tier uses its own complete block when present
    /// (`review.fast` / `review.deep`), else falls back to the flat `review.*` block — so a values file
    /// with no tier blocks (the legacy shape) keeps working, with both tiers on the flat config. The
    /// returned FAST config carries the structural `fast` flag (single diff-only turn, no retrieval); the
    /// DEEP config does not. Transition-safe: this runner accepts BOTH the flat and the nested shapes, so
    /// it can deploy before the values are restructured.
    pub fn resolve_tiers(file: Option<&FileConfig>) -> anyhow::Result<ReviewConfigs> {
        let Some(r) = file.and_then(|f| f.review.as_ref()) else {
            // Local/dev env path: one config from env, used for both tiers.
            let deep = Self::from_env()?;
            let mut fast = deep.clone();
            if let Some(c) = fast.as_mut() {
                c.fast = true;
            }
            return Ok(ReviewConfigs { fast, deep });
        };
        let deep = match r.deep.as_deref() {
            Some(d) => Self::from_review_file(d)?,
            None => Self::from_review_file(r)?,
        };
        let mut fast = match r.fast.as_deref() {
            Some(f) => Self::from_review_file(f)?,
            None => Self::from_review_file(r)?,
        };
        if let Some(c) = fast.as_mut() {
            c.fast = true;
        }
        Ok(ReviewConfigs { fast, deep })
    }
}

/// Resolved review configs for both tiers (ADR-0062). The runner picks one per task by its tier; each
/// is a complete, independent config (own model/gateway/prompt/budget). Either side is `None` when that
/// tier's model is empty (review disabled).
#[derive(Debug, Clone)]
pub struct ReviewConfigs {
    pub fast: Option<ReviewConfig>,
    pub deep: Option<ReviewConfig>,
}

impl ReviewConfigs {
    /// The config to run a task of this tier: `fast` → the fast config, anything else → deep. Returns
    /// `None` when that tier is disabled (no model). The selected fast config already carries the
    /// structural `fast` flag set by [`ReviewConfig::resolve_tiers`].
    pub fn for_tier(&self, tier: &str) -> Option<&ReviewConfig> {
        match tier {
            "fast" => self.fast.as_ref(),
            _ => self.deep.as_ref(),
        }
    }
}

/// Load the **required** reviewer system prompt (ADR-0037): the `system_prompt_file` template when a
/// path is given, else the `REVIEW_SYSTEM_PROMPT` env. Errors if neither yields a non-empty prompt —
/// there is deliberately no built-in default, so a misconfigured deploy fails the review closed
/// instead of silently running a weak fallback. The prompt is owned in ai-helm
/// (`config.reviewSystemPrompt`) and mounted into the Job.
fn require_system_prompt(system_prompt_file: Option<&str>) -> anyhow::Result<String> {
    let from_file = match system_prompt_file {
        Some(path) if !path.trim().is_empty() => Some(
            lightbridge_config::load_template(Path::new(path))
                .with_context(|| format!("loading review.system_prompt_file {path}"))?,
        ),
        _ => None,
    };
    let prompt = from_file
        .or_else(|| std::env::var("REVIEW_SYSTEM_PROMPT").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    prompt.context(
        "no review system prompt configured: set review.system_prompt_file (mounted from the \
         ai-helm `config.reviewSystemPrompt` ConfigMap) or REVIEW_SYSTEM_PROMPT. There is no \
         built-in default (ADR-0037) — review fails closed without one.",
    )
}

/// Parse a boolean env var (`1`/`true`/`yes`/`on` = true; `0`/`false`/`no`/`off` = false), returning
/// `None` when unset/empty/unrecognized so the caller applies its own default.
fn parse_env_bool(name: &str) -> Option<bool> {
    match std::env::var(name)
        .ok()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Resolved configuration for the deterministic SAST pass (ADR-0061). Absent ⇒ SAST disabled, so this is
/// produced as an `Option` like [`ReviewConfig`]. Unlike the LLM config there is no fail-closed path: a
/// half-set SAST block falls back to the defaults, because SAST is a best-effort *additive* signal whose
/// absence must never fail a review.
#[derive(Debug, Clone)]
pub struct SastConfig {
    /// opengrep binary name/path.
    pub bin: String,
    /// `--config` value (a vendored local rules dir by default).
    pub rules: String,
    /// Minimum SARIF level to surface (`error`|`warning`|`note`).
    pub min_severity: String,
    /// Cap on findings posted per review (excess logged, not silently dropped).
    pub max_findings: usize,
    /// Wall-clock ceiling on one scan, seconds.
    pub timeout_secs: u64,
}

impl SastConfig {
    /// Resolve from the file config's `sast` block when present, else env (`SAST_*`). Returns `None`
    /// (SAST disabled) unless `enabled` is explicitly true — opt-in so the feature lights up only once
    /// the opengrep-bearing image is deployed and an operator turns it on.
    pub fn resolve(file: Option<&FileConfig>) -> Option<Self> {
        let f = file.and_then(|f| f.sast.as_ref());
        let enabled = f
            .and_then(|s| s.enabled)
            .or_else(|| parse_env_bool("SAST_ENABLED"))
            .unwrap_or(false);
        if !enabled {
            return None;
        }
        let pick = |file_val: Option<&String>, env: &str, default: &str| -> String {
            file_val
                .map(|s| s.to_string())
                .or_else(|| std::env::var(env).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Some(Self {
            bin: pick(f.and_then(|s| s.bin.as_ref()), "SAST_BIN", DEFAULT_SAST_BIN),
            rules: pick(
                f.and_then(|s| s.rules.as_ref()),
                "SAST_RULES",
                DEFAULT_SAST_RULES,
            ),
            min_severity: pick(
                f.and_then(|s| s.min_severity.as_ref()),
                "SAST_MIN_SEVERITY",
                DEFAULT_SAST_MIN_SEVERITY,
            ),
            max_findings: f
                .and_then(|s| s.max_findings)
                .or_else(|| parse_env_u64("SAST_MAX_FINDINGS").map(|n| n as usize))
                .unwrap_or(DEFAULT_SAST_MAX_FINDINGS)
                .max(1),
            timeout_secs: f
                .and_then(|s| s.timeout_secs)
                .or_else(|| parse_env_u64("SAST_TIMEOUT_SECS"))
                .unwrap_or(DEFAULT_SAST_TIMEOUT_SECS)
                .max(1),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_config_requires_a_system_prompt() {
        // ADR-0037: no built-in default. With no prompt source set, resolving review fails closed.
        // (Guard against env bleed from a parallel test by asserting the error mentions the prompt.)
        std::env::remove_var("REVIEW_SYSTEM_PROMPT");
        let file = FileConfig {
            embeddings: None,
            sast: None,
            review: Some(ReviewFile {
                base_url: "https://gw/v1".to_string(),
                api_key: "k".to_string(),
                model: "m".to_string(),
                system_prompt_file: None,
                max_diff_chars: None,
                temperature: None,
                top_p: None,
                max_tokens: None,
                extra: None,
                request_timeout_secs: None,
                max_retries: None,
                circuit_breaker_threshold: None,
                max_turns: None,
                max_batch_size: None,
                max_files_read: None,
                max_searches: None,
                max_batches: None,
                context_window: None,
                stream: None,
                fast: None,
                deep: None,
                tools: None,
            }),
        };
        let err =
            ReviewConfig::resolve(Some(&file)).expect_err("must fail closed without a prompt");
        assert!(
            format!("{err:#}").contains("system prompt"),
            "error names the missing prompt: {err:#}"
        );
    }

    // A zero turn/budget (explicitly set in config or env) must clamp to ≥1, not silently no-op the
    // run (`for turn in 0..0`) or pre-disable a tool (`count >= 0`). #180 dogfood-review catch.
    #[test]
    fn turn_and_read_budgets_clamp_to_at_least_one() {
        // A temp prompt file so resolve() succeeds without touching env (parallel-safe).
        let prompt = std::env::temp_dir().join(format!("lci-clamp-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let file = FileConfig {
            embeddings: None,
            sast: None,
            review: Some(ReviewFile {
                base_url: "https://gw/v1".to_string(),
                api_key: "k".to_string(),
                model: "m".to_string(),
                system_prompt_file: Some(prompt.to_string_lossy().into_owned()),
                max_diff_chars: None,
                temperature: None,
                top_p: None,
                max_tokens: None,
                extra: None,
                request_timeout_secs: None,
                max_retries: None,
                circuit_breaker_threshold: None,
                // Every count knob explicitly 0 — `Some(0)` short-circuits the env fallback, so this is
                // deterministic regardless of the test environment.
                max_turns: Some(0),
                max_batch_size: Some(0),
                max_files_read: Some(0),
                max_searches: Some(0),
                max_batches: Some(0),
                // A 0 context window is meaningless — it must resolve to "disabled" (None), not a
                // window of zero that would force wind-down on turn 0 (ADR-0045).
                context_window: Some(0),
                stream: None,
                fast: None,
                deep: None,
                tools: None,
            }),
        };
        let cfg = ReviewConfig::resolve(Some(&file))
            .expect("resolves with a prompt file")
            .expect("review enabled");
        assert_eq!(cfg.max_turns, 1, "max_turns clamped");
        assert_eq!(cfg.max_batch_size, 1, "max_batch_size clamped");
        assert_eq!(
            cfg.context_window, None,
            "a 0 context window resolves to disabled"
        );
        assert_eq!(cfg.max_files_read, 1, "max_files_read clamped");
        assert_eq!(cfg.max_searches, 1, "max_searches clamped");
        assert_eq!(cfg.max_batches, 1, "max_batches clamped");
        std::fs::remove_file(&prompt).ok();
    }

    /// A minimal review block with the given model (and a shared prompt file), every other knob unset.
    #[cfg(test)]
    fn review_block(model: &str, prompt_file: &str) -> ReviewFile {
        ReviewFile {
            base_url: "https://gw/v1".to_string(),
            api_key: "k".to_string(),
            model: model.to_string(),
            system_prompt_file: Some(prompt_file.to_string()),
            max_diff_chars: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            extra: None,
            request_timeout_secs: None,
            max_retries: None,
            circuit_breaker_threshold: None,
            max_turns: None,
            max_batch_size: None,
            max_files_read: None,
            max_searches: None,
            max_batches: None,
            context_window: None,
            stream: None,
            fast: None,
            deep: None,
            tools: None,
        }
    }

    // Two-tier review (ADR-0062): with `review.fast`/`review.deep` present, each tier resolves to its own
    // complete config (own model); the fast config carries the structural `fast` flag, deep does not.
    #[test]
    fn resolve_tiers_uses_independent_per_tier_blocks() {
        let prompt = std::env::temp_dir().join(format!("lci-tiers-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let p = prompt.to_string_lossy().into_owned();
        let mut flat = review_block("flat-model", &p);
        flat.fast = Some(Box::new(review_block("fast-model", &p)));
        flat.deep = Some(Box::new(review_block("deep-model", &p)));
        let file = FileConfig {
            embeddings: None,
            sast: None,
            review: Some(flat),
        };
        let tiers = ReviewConfig::resolve_tiers(Some(&file)).expect("resolves");
        let fast = tiers.for_tier("fast").expect("fast enabled");
        let deep = tiers.for_tier("deep").expect("deep enabled");
        assert_eq!(fast.model, "fast-model");
        assert!(fast.fast, "fast tier carries the structural fast flag");
        assert_eq!(deep.model, "deep-model");
        assert!(!deep.fast, "deep tier is the full run");
        std::fs::remove_file(&prompt).ok();
    }

    // Back-compat: a flat `review.*` block with NO tier sub-blocks resolves both tiers to the flat config
    // (the fast one still flagged). This is the transition shape — the runner deploys before the values
    // are restructured.
    #[test]
    fn resolve_tiers_falls_back_to_flat_block() {
        let prompt = std::env::temp_dir().join(format!("lci-flat-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let p = prompt.to_string_lossy().into_owned();
        let file = FileConfig {
            embeddings: None,
            sast: None,
            review: Some(review_block("only-model", &p)),
        };
        let tiers = ReviewConfig::resolve_tiers(Some(&file)).expect("resolves");
        let fast = tiers.for_tier("fast").expect("fast enabled");
        let deep = tiers.for_tier("deep").expect("deep enabled");
        assert_eq!(
            fast.model, "only-model",
            "fast falls back to the flat block"
        );
        assert_eq!(
            deep.model, "only-model",
            "deep falls back to the flat block"
        );
        assert!(fast.fast && !deep.fast, "flags still set per tier");
        std::fs::remove_file(&prompt).ok();
    }

    // Per-tier tool allowlist (ADR-0062): a valid list resolves through to the tier config; an EMPTY
    // list fails closed (a tier with no tools can't act). Unknown names are rejected earlier, at
    // deserialize, by the closed `ReviewTool` enum (see `unknown_tool_name_fails_at_deserialize`).
    #[test]
    fn resolve_tools_allowlist_carries_through_and_rejects_empty() {
        let prompt = std::env::temp_dir().join(format!("lci-tools-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let p = prompt.to_string_lossy().into_owned();

        // A good allowlist resolves and is carried onto the config.
        let mut good = review_block("m", &p);
        good.tools = Some(vec![
            ReviewTool::AddReviewComment,
            ReviewTool::Finish,
            ReviewTool::Abort,
        ]);
        let cfg = ReviewConfig::from_review_file(&good)
            .expect("resolves")
            .expect("enabled");
        assert_eq!(
            cfg.tools.as_deref(),
            Some(
                [
                    ReviewTool::AddReviewComment,
                    ReviewTool::Finish,
                    ReviewTool::Abort
                ]
                .as_slice()
            )
        );

        // An empty list is rejected — a tier with no tools can't act.
        let mut empty = review_block("m", &p);
        empty.tools = Some(vec![]);
        ReviewConfig::from_review_file(&empty).expect_err("empty allowlist must fail");

        std::fs::remove_file(&prompt).ok();
    }

    // The closed enum rejects an unknown tool name at parse time — serde names the offending value, so a
    // typo in `review.<tier>.tools` fails the config loudly instead of silently offering fewer tools.
    #[test]
    fn unknown_tool_name_fails_at_deserialize() {
        let json = r#"{"review":{"base_url":"u","api_key":"k","model":"m",
                        "tools":["add_review_comment","nope_tool"]}}"#;
        let err =
            serde_json::from_str::<FileConfig>(json).expect_err("unknown tool must fail parsing");
        assert!(
            err.to_string().contains("nope_tool") || err.to_string().contains("unknown variant"),
            "serde names the bad tool: {err}"
        );
    }

    // Drift guard: the operator-facing `ReviewTool` enum must stay in lockstep with the tool surface the
    // agent actually dispatches (`tools::known_tool_names`). Add/remove a tool without updating the enum
    // and an allowlist would filter against a stale set — this fails the build instead.
    #[test]
    fn review_tool_enum_matches_the_dispatch_surface() {
        use std::collections::BTreeSet;
        let enum_names: BTreeSet<&str> = ReviewTool::ALL.iter().map(|t| t.as_str()).collect();
        let known: BTreeSet<&str> = crate::review::native::tools::known_tool_names()
            .into_iter()
            .collect();
        assert_eq!(
            enum_names, known,
            "ReviewTool variants must match tools::known_tool_names() exactly"
        );
    }

    // `review.stream` (file config) takes precedence; when unset it falls back to the `LLM_STREAM`
    // env. Here the file sets it explicitly so the result is deterministic regardless of the ambient
    // env (#206 streaming toggle, promoted from env-only to a config knob).
    #[test]
    fn review_stream_from_file_wins() {
        let prompt = std::env::temp_dir().join(format!("lci-stream-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let file = FileConfig {
            embeddings: None,
            sast: None,
            review: Some(ReviewFile {
                base_url: "https://gw/v1".into(),
                api_key: "k".into(),
                model: "m".into(),
                system_prompt_file: Some(prompt.to_string_lossy().into_owned()),
                stream: Some(true),
                ..Default::default()
            }),
        };
        let cfg = ReviewConfig::resolve(Some(&file))
            .expect("resolves with a prompt file")
            .expect("review enabled");
        assert!(cfg.stream, "review.stream=true is honoured over the env");
        std::fs::remove_file(&prompt).ok();
    }
}
