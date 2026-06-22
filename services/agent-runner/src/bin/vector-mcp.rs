//! `lightbridge-vector-mcp` — a stdio MCP server exposing semantic search over the task's pgvector
//! index (epic #5, slice 4b; ADR-0020). OpenCode spawns it; it embeds the query (with the runner's
//! embeddings key) and calls the control plane's scoped `search` endpoint. It holds the embeddings
//! key + the runner bearer — **no** datastore credentials (those stay in the control plane).

use std::sync::Arc;

use agent_runner::bootstrap::client::ControlPlaneClient;
use agent_runner::bootstrap::config::{EmbeddingsConfig, RunnerConfig};
use agent_runner::indexer::embeddings::EmbeddingsClient;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router, ServiceExt};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SemanticSearchRequest {
    #[schemars(description = "Natural-language or code query to search the repository for")]
    query: String,
    #[schemars(description = "Maximum number of results (default 10, max 100)")]
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Clone)]
struct VectorMcp {
    client: Arc<ControlPlaneClient>,
    embedder: Arc<EmbeddingsClient>,
    task_id: Uuid,
}

#[tool_router(server_handler)]
impl VectorMcp {
    #[tool(
        description = "Semantic search over the repository's indexed code by meaning (pgvector). \
                       Returns the most similar code chunks with file path, line range, and score."
    )]
    async fn lightbridge_vector_semantic_search(
        &self,
        Parameters(req): Parameters<SemanticSearchRequest>,
    ) -> String {
        match self.run(&req.query, req.limit.unwrap_or(10)).await {
            Ok(json) => json,
            // MCP tool errors are returned to the model as text, not transport failures, so it can
            // recover (retry, rephrase) rather than aborting the session.
            Err(error) => format!("error: {error:#}"),
        }
    }
}

impl VectorMcp {
    async fn run(&self, query: &str, limit: i64) -> anyhow::Result<String> {
        let mut vectors = self.embedder.embed(&[query]).await?;
        let embedding = vectors
            .pop()
            .ok_or_else(|| anyhow::anyhow!("embeddings API returned no vector"))?;
        let hits = self.client.search(self.task_id, &embedding, limit).await?;
        Ok(serde_json::to_string_pretty(&hits)?)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // MCP speaks JSON-RPC on stdout — logs MUST go to stderr or they corrupt the protocol stream.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = RunnerConfig::from_env()?;
    let embeddings = EmbeddingsConfig::from_env()?;
    let server = VectorMcp {
        client: Arc::new(ControlPlaneClient::new(
            &config.control_plane_url,
            &config.runner_token,
        )),
        embedder: Arc::new(EmbeddingsClient::new(
            &embeddings.base_url,
            &embeddings.api_key,
            &embeddings.model,
        )),
        task_id: config.task_id,
    };

    tracing::info!(task_id = %config.task_id, "lightbridge-vector-mcp starting");
    let service = rmcp::transport::stdio();
    server.serve(service).await?.waiting().await?;
    Ok(())
}
