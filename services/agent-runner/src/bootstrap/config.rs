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

/// One review model's tuning (ADR-0051): the per-request knobs that follow whichever model issues a
/// turn. Used for both the primary (`review.config`) and the fallback (`review.fallback.config`).
/// All optional + numeric-string tolerant; unset inherits the runner default (primary) or the
/// primary's effective value (fallback).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelTuningFile {
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub temperature: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_f64")]
    pub top_p: Option<f64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_i64")]
    pub max_tokens: Option<i64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub request_timeout_secs: Option<u64>,
    #[serde(default, deserialize_with = "lightbridge_config::de::opt_u64")]
    pub max_retries: Option<u64>,
}

/// The fallback review model (ADR-0051): its own model id + tuning, used on per-turn failover
/// (ADR-0039). Replaces the bare `fallback_model` string so the fallback carries its OWN context
/// window, generation params, and timeout instead of silently inheriting the primary's.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FallbackFile {
    pub model: String,
    #[serde(default)]
    pub config: Option<ModelTuningFile>,
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
    /// Optional secondary model to fail over to when the primary exhausts its retries on a turn
    /// (ADR-0039). **Deprecated** by `fallback` below (ADR-0051) — kept for dual-read so an older
    /// agent.json still parses; ignored when `fallback` is present.
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// The fallback model with its OWN tuning (ADR-0051): own context window, generation params, and
    /// timeout. Takes precedence over `fallback_model`. Unset (and no `fallback_model`) = no failover.
    #[serde(default)]
    pub fallback: Option<FallbackFile>,
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
    /// merged verbatim into the chat body. Empty = nothing extra. (Primary model; the fallback gets
    /// its own in a follow-up.)
    pub extra: serde_json::Map<String, serde_json::Value>,
    /// Resilience policy for the LLM transport: timeout, retry/backoff, circuit breaker (ADR-0039).
    /// Always present (defaults applied at resolve time). These are the **primary** model's per-request
    /// knobs + the loop-level breaker.
    pub resilience: ResilienceConfig,
    /// The fallback review model with its OWN per-request config (ADR-0051): own context window,
    /// generation params, timeout, and retries, applied on a per-turn failover (ADR-0039). `None` =
    /// single-model behaviour (no failover). Loop-cumulative budgets stay the primary's.
    pub fallback: Option<FallbackConfig>,
}

/// The resolved fallback review model (ADR-0051). Each tuning field defaults to the **primary's**
/// effective value when the operator didn't set it, so a fallback configured with only a model id
/// behaves exactly as the old `fallback_model` did (inherit the primary), while an operator can now
/// override the window / params / timeout per the `-pro` model's real characteristics.
/// The trim/context-budget window is deliberately NOT here: it's a pre-turn, run-level decision keyed
/// on the primary's `context_window` (the loop can't know a turn will fail over before it sends). A
/// fallback with a smaller window is backstopped by the ADR-0045 tier-1 overflow-finalize, not a
/// per-fallback trim. So the fallback's per-request knobs are its generation params + timeout/retries.
#[derive(Debug, Clone)]
pub struct FallbackConfig {
    pub model: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub request_timeout_secs: u64,
    pub max_retries: u32,
}

/// Resilience policy for the review LLM transport (ADR-0039). eaig can legitimately take ~2 minutes
/// per turn, so the timeout is deliberately generous; retries are bounded and only fire on transient
/// failures; a per-run circuit breaker fails fast before the turn budget is exhausted; an optional
/// secondary model provides config-gated failover.
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
            resilience: ResilienceConfig::from_env(),
            // Env path: `LLM_FALLBACK_MODEL` (a bare id) inherits the primary's env tuning — there are
            // no per-fallback env vars. The file path (resolve) is where per-model tuning lives.
            fallback: std::env::var("LLM_FALLBACK_MODEL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|model| FallbackConfig {
                    model,
                    temperature: None,
                    top_p: None,
                    max_tokens: None,
                    request_timeout_secs: parse_env_u64("LLM_REQUEST_TIMEOUT_SECS")
                        .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS),
                    max_retries: parse_env_u64("LLM_MAX_RETRIES")
                        .map(|n| n as u32)
                        .unwrap_or(DEFAULT_MAX_RETRIES),
                }),
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
        // Primary effective values (file wins, else env, else default). Computed as locals so the
        // fallback can default each of its per-request knobs to the primary's (ADR-0051).
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
        let max_retries = r
            .max_retries
            .or_else(|| parse_env_u64("LLM_MAX_RETRIES"))
            .map(|n| n as u32)
            .unwrap_or(DEFAULT_MAX_RETRIES);
        let fallback = resolve_fallback(
            r,
            temperature,
            top_p,
            max_tokens,
            request_timeout_secs,
            max_retries,
        );
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
            resilience: ResilienceConfig {
                request_timeout_secs,
                max_retries,
                circuit_breaker_threshold: r
                    .circuit_breaker_threshold
                    .or_else(|| parse_env_u64("LLM_CIRCUIT_BREAKER_THRESHOLD"))
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_CIRCUIT_BREAKER_THRESHOLD),
            },
            fallback,
        }))
    }
}

/// Resolve the fallback model (ADR-0051) from a `review` file block, defaulting each per-request knob
/// to the primary's effective value (`p_*`). Prefers the nested `fallback { model, config }`; falls
/// back to the deprecated `fallback_model` string (then `LLM_FALLBACK_MODEL`), which inherits all of
/// the primary's tuning — i.e. exactly the pre-0051 behaviour. `None` = no failover.
fn resolve_fallback(
    r: &ReviewFile,
    p_temperature: Option<f64>,
    p_top_p: Option<f64>,
    p_max_tokens: Option<i64>,
    p_request_timeout_secs: u64,
    p_max_retries: u32,
) -> Option<FallbackConfig> {
    if let Some(fb) = r.fallback.as_ref().filter(|fb| !fb.model.trim().is_empty()) {
        let c = fb.config.as_ref();
        return Some(FallbackConfig {
            model: fb.model.trim().to_string(),
            temperature: c.and_then(|c| c.temperature).or(p_temperature),
            top_p: c.and_then(|c| c.top_p).or(p_top_p),
            max_tokens: c.and_then(|c| c.max_tokens).or(p_max_tokens),
            request_timeout_secs: c
                .and_then(|c| c.request_timeout_secs)
                .unwrap_or(p_request_timeout_secs),
            max_retries: c
                .and_then(|c| c.max_retries)
                .map(|n| n as u32)
                .unwrap_or(p_max_retries),
        });
    }
    // Deprecated path: a bare `fallback_model` string (or env) inherits all the primary's tuning.
    let model = r
        .fallback_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("LLM_FALLBACK_MODEL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })?;
    Some(FallbackConfig {
        model,
        temperature: p_temperature,
        top_p: p_top_p,
        max_tokens: p_max_tokens,
        request_timeout_secs: p_request_timeout_secs,
        max_retries: p_max_retries,
    })
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
                fallback_model: None,
                fallback: None,
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
                fallback_model: None,
                fallback: None,
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

    // ADR-0051: the fallback gets its OWN per-request config — overrides what it sets, inherits the
    // primary's effective value for what it doesn't; and a legacy bare `fallback_model` inherits all.
    #[test]
    fn fallback_inherits_primary_then_overrides() {
        let prompt = std::env::temp_dir().join(format!("lci-fb-{}.md", std::process::id()));
        std::fs::write(&prompt, "You are a reviewer.").unwrap();
        let prompt_path = prompt.to_string_lossy().into_owned();

        // Nested fallback: overrides max_tokens + timeout, inherits temperature from primary.
        let nested = FileConfig {
            embeddings: None,
            review: Some(ReviewFile {
                base_url: "https://gw/v1".into(),
                api_key: "k".into(),
                model: "primary".into(),
                system_prompt_file: Some(prompt_path.clone()),
                context_window: Some(100_000),
                temperature: Some(0.5),
                request_timeout_secs: Some(180),
                fallback: Some(FallbackFile {
                    model: "fb".into(),
                    config: Some(ModelTuningFile {
                        request_timeout_secs: Some(240),
                        max_tokens: Some(4096),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            }),
        };
        let fb = ReviewConfig::resolve(Some(&nested))
            .unwrap()
            .unwrap()
            .fallback
            .expect("fallback present");
        assert_eq!(fb.model, "fb");
        assert_eq!(fb.max_tokens, Some(4096), "overridden");
        assert_eq!(fb.request_timeout_secs, 240, "overridden");
        assert_eq!(fb.temperature, Some(0.5), "inherited from primary");

        // Legacy bare `fallback_model` (dual-read) inherits ALL the primary's tuning.
        let legacy = FileConfig {
            embeddings: None,
            review: Some(ReviewFile {
                base_url: "https://gw/v1".into(),
                api_key: "k".into(),
                model: "primary".into(),
                system_prompt_file: Some(prompt_path),
                context_window: Some(100_000),
                temperature: Some(0.5),
                fallback_model: Some("old-fb".into()),
                ..Default::default()
            }),
        };
        let fb2 = ReviewConfig::resolve(Some(&legacy))
            .unwrap()
            .unwrap()
            .fallback
            .expect("legacy fallback present");
        assert_eq!(fb2.model, "old-fb");
        assert_eq!(
            fb2.temperature,
            Some(0.5),
            "legacy inherits primary temperature"
        );

        std::fs::remove_file(&prompt).ok();
    }
}
