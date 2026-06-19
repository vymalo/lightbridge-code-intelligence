//! Neo4j (Bolt) persistence for the structural code graph (ADR-0019).
//!
//! The agent runner spawns Graphify to produce a `graph.json` (symbols + `contains`/`method`/`calls`
//! edges) and POSTs it to the internal API; this module writes it to Neo4j. The control plane owns
//! the Neo4j credentials so the untrusted per-task Job never holds them (trust boundary, ADR-0002) —
//! the same reason chunk ingestion routes through the control plane rather than direct DB access.

use neo4rs::{query, Graph};

/// One graph node submitted by the runner (mirrors a Graphify `graph.json` node).
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub node_id: String,
    pub label: String,
    pub source_file: String,
    /// 1-based start line (Graphify emits `"L42"`; the runner parses the integer).
    pub start_line: i64,
}

/// One directed edge (`contains` / `method` / `calls` / …).
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
}

/// Connect to Neo4j from `NEO4J_URI` / `NEO4J_USER` / `NEO4J_PASSWORD`. Returns `Ok(None)` when
/// `NEO4J_URI` is unset (the graph endpoint then fails closed with 503, like the DB-less paths).
pub async fn connect_from_env() -> anyhow::Result<Option<Graph>> {
    use anyhow::Context;
    let uri = match std::env::var("NEO4J_URI") {
        Ok(uri) if !uri.is_empty() => uri,
        _ => return Ok(None),
    };
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".to_string());
    let pass = std::env::var("NEO4J_PASSWORD").unwrap_or_default();
    let graph = Graph::new(&uri, &user, &pass)
        .await
        .context("connecting to Neo4j")?;
    tracing::info!(%uri, "neo4j connected");
    Ok(Some(graph))
}

/// Upsert a repository snapshot's graph. Nodes and edges are scoped by `(repository_id, commit_sha)`
/// so re-indexing the same commit is idempotent and different commits coexist. Runs in one
/// transaction; returns `(nodes_written, edges_written)`.
///
/// Nodes are a generic `:Symbol` and edges a generic `[:REL {relation}]` — Cypher can't parameterize
/// labels/relationship types, and a property keeps the write a single prepared statement. Per-row
/// MERGE in a transaction is correct and simple; batching via `UNWIND` is a later optimization.
pub async fn upsert_graph(
    graph: &Graph,
    repository_id: i64,
    commit_sha: &str,
    nodes: &[GraphNode],
    edges: &[GraphEdge],
) -> anyhow::Result<(usize, usize)> {
    use anyhow::Context;
    let mut txn = graph.start_txn().await.context("begin neo4j txn")?;

    for n in nodes {
        txn.run(
            query(
                "MERGE (s:Symbol {repo_id: $repo, commit: $commit, node_id: $id}) \
                 SET s.label = $label, s.source_file = $file, s.start_line = $line",
            )
            .param("repo", repository_id)
            .param("commit", commit_sha)
            .param("id", n.node_id.as_str())
            .param("label", n.label.as_str())
            .param("file", n.source_file.as_str())
            .param("line", n.start_line),
        )
        .await
        .context("merge symbol node")?;
    }

    for e in edges {
        txn.run(
            query(
                "MATCH (a:Symbol {repo_id: $repo, commit: $commit, node_id: $src}) \
                 MATCH (b:Symbol {repo_id: $repo, commit: $commit, node_id: $dst}) \
                 MERGE (a)-[r:REL {relation: $rel}]->(b)",
            )
            .param("repo", repository_id)
            .param("commit", commit_sha)
            .param("src", e.source.as_str())
            .param("dst", e.target.as_str())
            .param("rel", e.relation.as_str()),
        )
        .await
        .context("merge edge")?;
    }

    txn.commit().await.context("commit neo4j txn")?;
    Ok((nodes.len(), edges.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live round-trip against a real Neo4j (the compose service). Ignored by default — CI has no
    /// Neo4j — run with `cargo test -p control-plane --ignored` after `docker compose up -d neo4j`.
    /// Proves the `neo4rs` API usage + Cypher actually execute and round-trip, scoped by commit.
    #[tokio::test]
    #[ignore = "requires a live Neo4j (docker compose up -d neo4j)"]
    async fn upsert_graph_round_trips_against_live_neo4j() {
        let uri =
            std::env::var("NEO4J_URI").unwrap_or_else(|_| "bolt://localhost:7687".to_string());
        let graph = Graph::new(&uri, "neo4j", "lightbridge")
            .await
            .expect("connect neo4j");

        let commit = "test-commit-graph";
        // Clean any prior run for this commit so the test is repeatable.
        graph
            .run(query("MATCH (s:Symbol {commit: $c}) DETACH DELETE s").param("c", commit))
            .await
            .expect("cleanup");

        let nodes = vec![
            GraphNode {
                node_id: "src_math_add".into(),
                label: "add()".into(),
                source_file: "src/math.rs".into(),
                start_line: 2,
            },
            GraphNode {
                node_id: "src_math_calc_bump".into(),
                label: "bump()".into(),
                source_file: "src/math.rs".into(),
                start_line: 6,
            },
        ];
        let edges = vec![GraphEdge {
            source: "src_math_calc_bump".into(),
            target: "src_math_add".into(),
            relation: "calls".into(),
        }];

        let (n, e) = upsert_graph(&graph, 42, commit, &nodes, &edges)
            .await
            .expect("upsert");
        assert_eq!((n, e), (2, 1));

        // Read the call edge back, scoped to this commit.
        let mut rows = graph
            .execute(
                query(
                    "MATCH (a:Symbol {commit: $c})-[r:REL {relation: 'calls'}]->(b:Symbol) \
                     RETURN a.node_id AS src, b.node_id AS dst",
                )
                .param("c", commit),
            )
            .await
            .expect("query");
        let row = rows.next().await.expect("a row").expect("row present");
        assert_eq!(row.get::<String>("src").unwrap(), "src_math_calc_bump");
        assert_eq!(row.get::<String>("dst").unwrap(), "src_math_add");

        // Idempotent: re-upserting the same commit doesn't duplicate.
        upsert_graph(&graph, 42, commit, &nodes, &edges)
            .await
            .expect("re-upsert");
        let mut count = graph
            .execute(query("MATCH (s:Symbol {commit: $c}) RETURN count(s) AS n").param("c", commit))
            .await
            .expect("count query");
        let c = count.next().await.expect("row").expect("present");
        assert_eq!(c.get::<i64>("n").unwrap(), 2, "MERGE is idempotent");

        // Cleanup.
        graph
            .run(query("MATCH (s:Symbol {commit: $c}) DETACH DELETE s").param("c", commit))
            .await
            .expect("final cleanup");
    }
}
