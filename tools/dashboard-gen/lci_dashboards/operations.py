"""Operations dashboard — RED-style metrics from Prometheus (the part Postgres/Loki can't give).

Metric names match the control plane's `metrics` instrumentation (see services/control-plane).
"""

from __future__ import annotations

from grafana_foundation_sdk.builders import dashboard, stat, timeseries

from .common import PROMETHEUS, Layout, promql

UID = "lci-operations"


def dashboard_builder() -> dashboard.Dashboard:
    layout = Layout()

    req_rate = (
        timeseries.Panel()
        .title("HTTP request rate")
        .datasource(PROMETHEUS)
        .unit("reqps")
        .with_target(
            promql("sum(rate(http_requests_total[$__rate_interval])) by (status)", legend="{{status}}")
        )
        .grid_pos(layout.place(12, 8))
    )
    req_latency = (
        timeseries.Panel()
        .title("HTTP p95 latency")
        .datasource(PROMETHEUS)
        .unit("s")
        .with_target(
            promql(
                "histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket"
                "[$__rate_interval])) by (le, path))",
                legend="{{path}}",
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    webhook_rate = (
        timeseries.Panel()
        .title("Webhook deliveries")
        .datasource(PROMETHEUS)
        .unit("ops")
        .with_target(
            promql(
                "sum(rate(lci_webhook_deliveries_total[$__rate_interval])) by (event)",
                legend="{{event}}",
            )
        )
        .grid_pos(layout.place(12, 8))
    )
    webhook_fail = (
        timeseries.Panel()
        .title("Webhook signature failures & duplicates")
        .datasource(PROMETHEUS)
        .unit("ops")
        .with_target(
            promql(
                "sum(rate(lci_webhook_signature_failures_total[$__rate_interval]))",
                ref_id="A",
                legend="signature failures",
            )
        )
        .with_target(
            promql(
                "sum(rate(lci_webhook_duplicate_deliveries_total[$__rate_interval]))",
                ref_id="B",
                legend="duplicates",
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    dispatch = (
        timeseries.Panel()
        .title("Dispatch outcomes")
        .datasource(PROMETHEUS)
        .unit("ops")
        .with_target(
            promql(
                "sum(rate(lci_dispatch_jobs_total[$__rate_interval])) by (outcome)",
                legend="{{outcome}}",
            )
        )
        .grid_pos(layout.place(12, 8))
    )
    claim_latency = (
        timeseries.Panel()
        .title("Job launch p95")
        .datasource(PROMETHEUS)
        .unit("s")
        .with_target(
            promql(
                "histogram_quantile(0.95, sum(rate(lci_dispatch_launch_seconds_bucket"
                "[$__rate_interval])) by (le))"
            )
        )
        .grid_pos(layout.place(12, 8))
    )

    tasks_created = (
        stat.Panel()
        .title("Tasks created ($__range)")
        .datasource(PROMETHEUS)
        .with_target(promql("sum(increase(lci_tasks_created_total[$__range]))"))
        .grid_pos(layout.place(24, 4))
    )

    return (
        dashboard.Dashboard("Lightbridge — Operations")
        .uid(UID)
        .tags(["lightbridge", "generated"])
        .refresh("30s")
        .time("now-6h", "now")
        .with_panel(req_rate)
        .with_panel(req_latency)
        .with_panel(webhook_rate)
        .with_panel(webhook_fail)
        .with_panel(dispatch)
        .with_panel(claim_latency)
        .with_panel(tasks_created)
    )
