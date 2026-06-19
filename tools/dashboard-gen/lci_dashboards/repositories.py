"""Repositories dashboard — forwards the web UI's repositories view (read-only)."""

from __future__ import annotations

from grafana_foundation_sdk.builders import dashboard, table

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

    return (
        dashboard.Dashboard("Lightbridge — Repositories")
        .uid(UID)
        .tags(["lightbridge", "generated"])
        .refresh("1m")
        .time("now-30d", "now")
        .with_panel(repos)
        .with_panel(indexes)
    )
