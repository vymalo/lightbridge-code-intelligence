"""Ingress & Dispatcher dashboard — webhook ingestion + queue/dispatch health.

Signature failures and duplicate deliveries are NOT in Postgres (duplicates are never inserted), so
those come from Loki log lines the control plane emits. Backlog, stuck-running tasks, attempts, and
recent dispatches come from Postgres.
"""

from __future__ import annotations

from grafana_foundation_sdk.builders import bargauge, dashboard, logs, stat, table, timeseries

from .common import LOKI, POSTGRES, Layout, logql, sql

UID = "lci-ingress-dispatcher"
DEFAULT_STREAM = '{app=~"lightbridge.*"}'


def dashboard_builder() -> dashboard.Dashboard:
    layout = Layout()

    stream_var = (
        dashboard.TextBoxVariable("stream").label("Loki stream").default_value(DEFAULT_STREAM)
    )

    backlog = (
        stat.Panel()
        .title("Queue backlog")
        .datasource(POSTGRES)
        .with_target(sql("SELECT count(*) FROM tasks WHERE status = 'queued'"))
        .grid_pos(layout.place(6, 4))
    )
    stuck = (
        stat.Panel()
        .title("Stuck running (lease expired)")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT count(*) FROM tasks "
                "WHERE status = 'running' AND lease_expires_at < now()"
            )
        )
        .grid_pos(layout.place(6, 4))
    )
    sig_fail = (
        stat.Panel()
        .title("Signature failures ($__range)")
        .datasource(LOKI)
        .with_target(logql('sum(count_over_time(${stream} |= "invalid webhook signature" [$__range]))'))
        .grid_pos(layout.place(6, 4))
    )
    dupes = (
        stat.Panel()
        .title("Duplicate deliveries ($__range)")
        .datasource(LOKI)
        .with_target(logql('sum(count_over_time(${stream} |= "duplicate delivery" [$__range]))'))
        .grid_pos(layout.place(6, 4))
    )

    deliveries = (
        timeseries.Panel()
        .title("Deliveries received")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT $__timeGroupAlias(received_at, $__interval), count(*) AS \"deliveries\" "
                "FROM github_deliveries WHERE $__timeFilter(received_at) GROUP BY 1 ORDER BY 1",
                fmt="time_series",
            )
        )
        .grid_pos(layout.place(12, 8))
    )
    attempts = (
        bargauge.Panel()
        .title("Task attempts distribution")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT attempts::text AS metric, count(*) AS value "
                "FROM tasks GROUP BY attempts ORDER BY attempts"
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    dispatched = (
        table.Panel()
        .title("Recently dispatched")
        .datasource(POSTGRES)
        .with_target(
            sql(
                "SELECT t.started_at AS \"dispatched\", t.job_name, t.status, t.attempts, "
                "t.lease_owner, coalesce(r.owner || '/' || r.name, t.repository_id::text) AS \"repository\" "
                "FROM tasks t LEFT JOIN repositories r ON r.id = t.repository_id "
                "WHERE t.job_name IS NOT NULL ORDER BY t.started_at DESC NULLS LAST LIMIT 100"
            )
        )
        .grid_pos(layout.place(24, 9))
    )

    errors = (
        logs.Panel()
        .title("Recent errors")
        .datasource(LOKI)
        .show_time(True)
        .wrap_log_message(True)
        .with_target(logql('${stream} | json | level = "ERROR"'))
        .grid_pos(layout.place(24, 10))
    )

    return (
        dashboard.Dashboard("Lightbridge — Ingress & Dispatcher")
        .uid(UID)
        .tags(["lightbridge", "generated"])
        .refresh("30s")
        .time("now-6h", "now")
        .with_variable(stream_var)
        .with_panel(backlog)
        .with_panel(stuck)
        .with_panel(sig_fail)
        .with_panel(dupes)
        .with_panel(deliveries)
        .with_panel(attempts)
        .with_panel(dispatched)
        .with_panel(errors)
    )
