//! Generation of the `opencode.json` the review agent runs under (ADR-0021).

use serde_json::{json, Value};

use crate::config::ReviewConfig;

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
pub fn opencode_config(review: &ReviewConfig, mcp_env: &[(String, String)]) -> Value {
    let env_obj: Value = mcp_env
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();

    let mcp_server = |bin: &str| {
        json!({
            "type": "local",
            "command": [bin],
            "enabled": true,
            "environment": env_obj,
        })
    };

    json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            super::PROVIDER_ID: {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Lightbridge Gateway (eaig)",
                "options": {
                    "baseURL": review.base_url,
                    "apiKey": "{env:LLM_API_KEY}",
                },
                "models": {
                    review.model.clone(): { "name": review.model.clone() }
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
            base_url: "https://gw.example/v1".to_string(),
            api_key: "secret".to_string(),
            model: "qwen-coder".to_string(),
        }
    }

    #[test]
    fn wires_provider_model_and_mcp_servers() {
        let env = vec![("TASK_ID".to_string(), "t1".to_string())];
        let cfg = opencode_config(&review(), &env);

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
