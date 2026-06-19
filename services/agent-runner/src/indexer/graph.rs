//! Structural graph indexing via Graphify (ADR-0019).
//!
//! Graphify is a standalone, multi-language (36 grammars) AST→graph extractor bundled into the
//! runner image. We spawn it headless (`graphify extract … --no-cluster`, no LLM/API key needed),
//! parse the `graph.json` it writes, and hand the nodes+edges to the control plane, which owns the
//! Neo4j write. We deliberately do *not* use Graphify for embeddings — it has none; the semantic
//! (pgvector) path stays with our own tree-sitter chunker.

use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::client::TaskContext;
use crate::client::{ControlPlaneClient, GraphBatch, GraphEdgePayload, GraphNodePayload};

/// Graphify's `graph.json` shape (only the fields we consume).
#[derive(Debug, Deserialize)]
struct GraphFile {
    #[serde(default)]
    nodes: Vec<RawNode>,
    #[serde(default)]
    edges: Vec<RawEdge>,
}

#[derive(Debug, Deserialize)]
struct RawNode {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    source_file: Option<String>,
    /// Graphify encodes the line as `"L42"`; may be absent for some node kinds.
    #[serde(default)]
    source_location: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawEdge {
    source: String,
    target: String,
    #[serde(default)]
    relation: String,
}

/// Run Graphify over the checkout, parse its graph, and submit it to the control plane.
/// Returns `(nodes, edges)` submitted. Best-effort: a Graphify failure is an error the caller can
/// log without failing the whole task (the semantic index may still have succeeded).
pub async fn index_graph(
    context: &TaskContext,
    checkout: &Path,
    client: &ControlPlaneClient,
) -> anyhow::Result<(usize, usize)> {
    let commit_sha = context
        .head_sha
        .as_deref()
        .unwrap_or(&context.default_branch)
        .to_string();

    let graph_json = run_graphify(checkout).await?;
    let parsed: GraphFile =
        serde_json::from_str(&graph_json).context("parsing graphify graph.json")?;

    let nodes: Vec<GraphNodePayload> = parsed
        .nodes
        .into_iter()
        .filter_map(|n| {
            // A node without a source file is a synthetic/aggregate node (e.g. a doc or community);
            // skip it — the structural code graph is what we want in Neo4j.
            let source_file = n.source_file?;
            Some(GraphNodePayload {
                node_id: n.id,
                label: n.label,
                source_file,
                start_line: parse_line(n.source_location.as_deref()),
            })
        })
        .collect();

    let edges: Vec<GraphEdgePayload> = parsed
        .edges
        .into_iter()
        .map(|e| GraphEdgePayload {
            source: e.source,
            target: e.target,
            relation: e.relation,
        })
        .collect();

    if nodes.is_empty() {
        tracing::info!("graphify produced no code nodes; skipping graph submit");
        return Ok((0, 0));
    }

    let (n, e) = (nodes.len(), edges.len());
    client
        .submit_graph(
            context.task_id,
            GraphBatch {
                commit_sha,
                nodes,
                edges,
            },
        )
        .await
        .context("submitting structural graph")?;
    tracing::info!(nodes = n, edges = e, "structural graph submitted");
    Ok((n, e))
}

/// Spawn `graphify extract <checkout> --no-cluster --out <out>` and return the graph.json contents.
async fn run_graphify(checkout: &Path) -> anyhow::Result<String> {
    let out_dir = checkout.join(".graphify-run");
    let status = tokio::process::Command::new("graphify")
        .arg("extract")
        .arg(checkout)
        .arg("--no-cluster")
        .arg("--out")
        .arg(&out_dir)
        // No semantic LLM pass — AST extraction only, fully offline. Be explicit so a stray
        // API key in the environment can't trigger paid calls.
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .status()
        .await
        .context("spawning graphify (is it on PATH in the image?)")?;

    if !status.success() {
        anyhow::bail!("graphify extract exited with {status}");
    }

    // `--out DIR` writes `DIR/graphify-out/graph.json`.
    let graph_path = out_dir.join("graphify-out").join("graph.json");
    tokio::fs::read_to_string(&graph_path)
        .await
        .with_context(|| format!("reading {}", graph_path.display()))
}

/// Parse Graphify's `"L42"` location into a 1-based line; defaults to 0 when absent/garbled.
fn parse_line(loc: Option<&str>) -> i64 {
    loc.and_then(|s| s.trim_start_matches(['L', 'l']).parse::<i64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_handles_graphify_format() {
        assert_eq!(parse_line(Some("L42")), 42);
        assert_eq!(parse_line(Some("L1")), 1);
        assert_eq!(parse_line(None), 0);
        assert_eq!(parse_line(Some("garbage")), 0);
    }

    #[test]
    fn parses_a_graphify_graph_json() {
        // A trimmed real sample (Rust + Python) from `graphify extract --no-cluster`.
        let json = r#"{
            "nodes": [
                {"id": "src_math_add", "label": "add()", "source_file": "src/math.rs", "source_location": "L2"},
                {"id": "src_app_greeter", "label": "Greeter", "source_file": "src/app.py", "source_location": "L4"},
                {"id": "community_0", "label": "Community 0"}
            ],
            "edges": [
                {"source": "src_app_greeter_hi", "target": "src_app_greet", "relation": "calls"}
            ]
        }"#;
        let parsed: GraphFile = serde_json::from_str(json).unwrap();
        // The synthetic community node (no source_file) is dropped; the two code nodes remain.
        let code_nodes: Vec<_> = parsed
            .nodes
            .into_iter()
            .filter(|n| n.source_file.is_some())
            .collect();
        assert_eq!(code_nodes.len(), 2);
        assert_eq!(parsed.edges[0].relation, "calls");
    }
}
