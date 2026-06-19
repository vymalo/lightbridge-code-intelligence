//! `lightbridge-graph-mcp` — a stdio MCP server exposing structural queries over the task's Neo4j
//! graph (epic #5, slice 4b; ADR-0020). OpenCode spawns it; it calls the control plane's scoped
//! `graph/query` endpoint. It holds only the runner bearer — no Neo4j credentials.

use std::sync::Arc;

use agent_runner::client::ControlPlaneClient;
use agent_runner::config::RunnerConfig;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router, ServiceExt};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct FindSymbolRequest {
    #[schemars(
        description = "Substring of a symbol name, node id, or file path (case-insensitive)"
    )]
    term: String,
    #[schemars(description = "Maximum number of results (default 10, max 100)")]
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct GetCallersRequest {
    #[schemars(description = "The node id of the target symbol (from find_symbol)")]
    node_id: String,
    #[schemars(description = "Maximum number of results (default 10, max 100)")]
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Clone)]
struct GraphMcp {
    client: Arc<ControlPlaneClient>,
    task_id: Uuid,
}

#[tool_router(server_handler)]
impl GraphMcp {
    #[tool(
        description = "Find symbols (functions, classes, methods) by name, node id, or file path \
                       substring. Returns matching nodes with their node id, label, and location."
    )]
    async fn lightbridge_graph_find_symbol(
        &self,
        Parameters(req): Parameters<FindSymbolRequest>,
    ) -> String {
        let result = self
            .client
            .graph_find_symbol(self.task_id, &req.term, req.limit.unwrap_or(10))
            .await;
        render(result)
    }

    #[tool(
        description = "Return the symbols that call a given symbol (reverse call graph). Pass a \
                       node id from find_symbol."
    )]
    async fn lightbridge_graph_get_callers(
        &self,
        Parameters(req): Parameters<GetCallersRequest>,
    ) -> String {
        let result = self
            .client
            .graph_get_callers(self.task_id, &req.node_id, req.limit.unwrap_or(10))
            .await;
        render(result)
    }
}

/// Render a query result as a JSON string, or a recoverable error message for the model.
fn render(result: anyhow::Result<Vec<agent_runner::client::SymbolHit>>) -> String {
    match result.and_then(|hits| Ok(serde_json::to_string_pretty(&hits)?)) {
        Ok(json) => json,
        Err(error) => format!("error: {error:#}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // MCP speaks JSON-RPC on stdout — logs MUST go to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = RunnerConfig::from_env()?;
    let server = GraphMcp {
        client: Arc::new(ControlPlaneClient::new(
            &config.control_plane_url,
            &config.runner_token,
        )),
        task_id: config.task_id,
    };

    tracing::info!(task_id = %config.task_id, "lightbridge-graph-mcp starting");
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
