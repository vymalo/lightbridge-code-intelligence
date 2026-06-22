//! Generation of the `opencode.json` the review agent runs under (ADR-0021).

use serde_json::{json, Value};

use crate::bootstrap::config::ReviewConfig;

/// Absolute paths to the bundled MCP server binaries (installed by the Dockerfile).
const VECTOR_MCP_BIN: &str = "/usr/local/bin/lightbridge-vector-mcp";
const GRAPH_MCP_BIN: &str = "/usr/local/bin/lightbridge-graph-mcp";

/// Build the `opencode.json` value: the eaig OpenAI-compatible provider, our two stdio MCP servers,
/// and a locked-down permission set (the reviewer must not edit files, run shell, or fetch the web —
/// it only reads via the MCP tools). `mcp_env` is injected into each MCP subprocess's environment.
///
/// Secrets are passed by `{env:…}` reference for the provider (so `LLM_API_KEY` isn't written to the
/// file); the MCP env is written literally since those values must reach the child processes and the
/// file lives only on the ephemeral Job disk.
pub fn opencode_config(
    review: &ReviewConfig,
    mcp_env: &[(String, String)],
    attribution: &[(String, String)],
) -> Value {
    let env_obj: Value = mcp_env
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();

    // Provider options: the gateway base URL + key, plus attribution headers (epic #89) so the Envoy
    // AI Gateway bills the review's token spend to the right project. @ai-sdk/openai-compatible
    // forwards `options.headers` on every request.
    let mut provider_options = serde_json::Map::new();
    provider_options.insert("baseURL".to_string(), json!(review.base_url));
    provider_options.insert("apiKey".to_string(), json!("{env:LLM_API_KEY}"));
    if !attribution.is_empty() {
        let headers: serde_json::Map<String, Value> = attribution
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        provider_options.insert("headers".to_string(), Value::Object(headers));
    }

    let mcp_server = |bin: &str| {
        json!({
            "type": "local",
            "command": [bin],
            "enabled": true,
            "environment": env_obj,
        })
    };

    // Per-model generation params (temperature/top_p/max_tokens), passed as the model's `options` to
    // the OpenAI-compatible provider. Only present when configured — otherwise the model's defaults.
    let mut model_entry = json!({ "name": review.model.clone() });
    let mut options = serde_json::Map::new();
    if let Some(temperature) = review.temperature {
        options.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = review.top_p {
        options.insert("top_p".to_string(), json!(top_p));
    }
    if let Some(max_tokens) = review.max_tokens {
        options.insert("max_tokens".to_string(), json!(max_tokens));
    }
    if !options.is_empty() {
        model_entry["options"] = Value::Object(options);
    }

    json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            super::PROVIDER_ID: {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Lightbridge Gateway (eaig)",
                "options": Value::Object(provider_options),
                "models": {
                    review.model.clone(): model_entry,
                }
            }
        },
        "model": format!("{}/{}", super::PROVIDER_ID, review.model),
        "mcp": {
            "lightbridge_vector": mcp_server(VECTOR_MCP_BIN),
            "lightbridge_graph": mcp_server(GRAPH_MCP_BIN),
        },
        // The reviewer reads only — no edits, shell, or web. Defense in depth on top of the MCP
        // tools being the only repo access (the untrusted repo can't trick it into running code).
        "permission": {
            "edit": "deny",
            "bash": "deny",
            "webfetch": "deny",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review() -> ReviewConfig {
        ReviewConfig {
            agent: crate::bootstrap::config::ReviewAgent::OpenCode,
            base_url: "https://gw.example/v1".to_string(),
            api_key: "secret".to_string(),
            model: "qwen-coder".to_string(),
            system_prompt: None,
            max_diff_chars: 60_000,
            temperature: Some(0.2),
            top_p: None,
            max_tokens: Some(4096),
        }
    }

    #[test]
    fn model_options_carry_configured_params() {
        let cfg = opencode_config(&review(), &[], &[]);
        let opts = &cfg["provider"]["eaig"]["models"]["qwen-coder"]["options"];
        assert_eq!(opts["temperature"], serde_json::json!(0.2));
        assert_eq!(opts["max_tokens"], serde_json::json!(4096));
        assert!(opts.get("top_p").is_none(), "unset params are omitted");
    }

    #[test]
    fn attribution_headers_go_to_provider_options() {
        let attribution = vec![(
            "x-code-intelligence-repo".to_string(),
            "octo/hello".to_string(),
        )];
        let cfg = opencode_config(&review(), &[], &attribution);
        assert_eq!(
            cfg["provider"]["eaig"]["options"]["headers"]["x-code-intelligence-repo"],
            "octo/hello"
        );
        // No attribution → no headers key.
        let bare = opencode_config(&review(), &[], &[]);
        assert!(bare["provider"]["eaig"]["options"].get("headers").is_none());
    }

    #[test]
    fn wires_provider_model_and_mcp_servers() {
        let env = vec![("TASK_ID".to_string(), "t1".to_string())];
        let cfg = opencode_config(&review(), &env, &[]);

        // Provider is the OpenAI-compatible gateway; the key is an env reference, not the literal.
        let provider = &cfg["provider"]["eaig"];
        assert_eq!(provider["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(provider["options"]["baseURL"], "https://gw.example/v1");
        assert_eq!(provider["options"]["apiKey"], "{env:LLM_API_KEY}");
        assert_ne!(
            provider["options"]["apiKey"], "secret",
            "key must not be inlined"
        );

        // Model is referenced as provider/model.
        assert_eq!(cfg["model"], "eaig/qwen-coder");

        // Both MCP servers wired to the bundled binaries, with the injected env.
        assert_eq!(
            cfg["mcp"]["lightbridge_vector"]["command"][0],
            VECTOR_MCP_BIN
        );
        assert_eq!(cfg["mcp"]["lightbridge_graph"]["command"][0], GRAPH_MCP_BIN);
        assert_eq!(
            cfg["mcp"]["lightbridge_graph"]["environment"]["TASK_ID"],
            "t1"
        );

        // Reviewer is read-only.
        assert_eq!(cfg["permission"]["edit"], "deny");
        assert_eq!(cfg["permission"]["bash"], "deny");
    }
}
