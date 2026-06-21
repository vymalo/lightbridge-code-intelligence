//! Repository data lifecycle (Epic #75, Milestone B).
//!
//! When a repository is removed from the installation or denied by an admin, its indexed data must be
//! purged so we don't retain code for repos nobody opted into: the semantic index (`code_chunks`),
//! the structural graph (Neo4j), the indexing-state bookkeeping (`repo_index`), and any in-flight
//! tasks (cancelled so the reaper stops their Jobs). The repository row itself is kept (status
//! `disabled`) for audit/history.

use std::sync::Arc;

use sqlx::PgPool;

use crate::AppState;

/// Purge all index data for a repository. Best-effort across stores: a failure in one (e.g. Neo4j
/// down) is logged and the rest still run, since this is reconciled on the next removal anyway. Safe
/// to run repeatedly (every delete is idempotent).
pub async fn purge_repository_data(
    pool: &PgPool,
    neo4j: Option<&neo4rs::Graph>,
    repository_id: i64,
) {
    let cancelled = match crate::db::cancel_active_tasks_for_repo(pool, repository_id).await {
        Ok(ids) => ids.len(),
        Err(error) => {
            tracing::warn!(%error, repository_id, "purge: cancel tasks failed");
            0
        }
    };
    let chunks = crate::db::delete_code_chunks_for_repo(pool, repository_id)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, repository_id, "purge: delete code_chunks failed");
            0
        });
    let _ = crate::db::delete_repo_index_rows(pool, repository_id)
        .await
        .map_err(|error| tracing::warn!(%error, repository_id, "purge: delete repo_index failed"));
    let nodes = match neo4j {
        Some(graph) => crate::neo4j::delete_repo_graph(graph, repository_id)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, repository_id, "purge: delete graph failed");
                0
            }),
        None => 0,
    };
    tracing::info!(
        repository_id,
        cancelled_tasks = cancelled,
        deleted_chunks = chunks,
        deleted_graph_nodes = nodes,
        "purged repository index data (repo removed/denied)"
    );
}

/// Spawn [`purge_repository_data`] so destructive cleanup never blocks a webhook (its ~10s budget) or
/// an admin request. No-op without a database.
pub fn spawn_purge(state: &AppState, repository_id: i64) {
    let Some(pool) = state.db.clone() else {
        return;
    };
    let neo4j: Option<Arc<neo4rs::Graph>> = state.neo4j.clone();
    tokio::spawn(async move {
        purge_repository_data(&pool, neo4j.as_deref(), repository_id).await;
    });
}
