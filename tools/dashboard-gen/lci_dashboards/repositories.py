"""Repositories dashboard — forwards the web UI's repositories view (read-only)."""

from __future__ import annotations

from grafana_foundation_sdk.builders import bargauge, dashboard, table

from .common import POSTGRES, Layout, sql

UID = "lci-repositories"


def dashboard_builder() -> dashboard.Dashboard:
    layout = Layout()

    repos = (
        table.Panel()
        .title("Repositories")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT r.owner || '/' || r.name AS \"repository\", r.default_branch AS \"branch\", "
                "r.active, count(t.id) AS \"tasks\", "
                "count(t.id) FILTER (WHERE t.status = 'failed') AS \"failed\", "
                "count(t.id) FILTER (WHERE t.status IN ('queued', 'running')) AS \"in_flight\", "
                "max(t.created_at) AS \"last_task\" "
                "FROM repositories r LEFT JOIN tasks t ON t.repository_id = r.id "
                "GROUP BY r.id, r.owner, r.name, r.default_branch, r.active "
                "ORDER BY count(t.id) DESC"
            )
        )
        .grid_pos(layout.place(24, 11))
    )

    indexes = (
        table.Panel()
        .title("Index status")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT r.owner || '/' || r.name AS \"repository\", ri.branch, ri.status, "
                "ri.commit_sha, ri.started_at, ri.completed_at "
                "FROM repo_index ri JOIN repositories r ON r.id = ri.repository_id "
                "ORDER BY ri.completed_at DESC NULLS LAST LIMIT 200"
            )
        )
        .grid_pos(layout.place(24, 11))
    )

    # Chunk counts for the most recently indexed commit per repo (DISTINCT ON by created_at), so a
    # re-index that supersedes an old commit doesn't double-count.
    index_size = (
        table.Panel()
        .title("Index size (latest commit)")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "WITH latest AS ("
                "  SELECT DISTINCT ON (repository_id) repository_id, commit_sha "
                "  FROM code_chunks ORDER BY repository_id, created_at DESC) "
                "SELECT r.owner || '/' || r.name AS \"repository\", "
                "left(cc.commit_sha, 8) AS \"commit\", "
                "count(*) AS \"chunks\", count(DISTINCT cc.file_path) AS \"files\" "
                "FROM code_chunks cc "
                "JOIN latest l ON l.repository_id = cc.repository_id AND l.commit_sha = cc.commit_sha "
                "JOIN repositories r ON r.id = cc.repository_id "
                "GROUP BY r.id, r.owner, r.name, cc.commit_sha "
                "ORDER BY count(*) DESC"
            )
        )
        .grid_pos(layout.place(12, 9))
    )
    by_language = (
        bargauge.Panel()
        .title("Indexed chunks by language")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT language AS metric, count(*) AS value "
                "FROM code_chunks GROUP BY language ORDER BY value DESC LIMIT 25"
            )
        )
        .grid_pos(layout.place(12, 9))
    )

    return (
        dashboard.Dashboard("Lightbridge — Repositories")
        .uid(UID)
        .tags(["lightbridge", "generated"])
        .refresh("1m")
        .time("now-30d", "now")
        .with_panel(repos)
        .with_panel(indexes)
        .with_panel(index_size)
        .with_panel(by_language)
    )
