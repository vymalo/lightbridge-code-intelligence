# Lightbridge observability chart

Deploys the Lightbridge Grafana dashboards via the [Grafana Operator](https://grafana.github.io/grafana-operator/):
a `GrafanaFolder`, the generated `GrafanaDashboard` CRs, and an optional read-only Postgres
`GrafanaDatasource`. Install into the `observability` namespace (or wherever your Grafana operator
watches).

Dashboards are **generated** by `tools/dashboard-gen` and committed under `dashboards/`. Edit the
Python, regenerate, commit — don't hand-edit the JSON.

## Before deploying — fill in `values.yaml`

| Value | What |
|---|---|
| `instanceSelector.matchLabels` | Labels selecting your Grafana instance (copy from a working dashboard, e.g. `alloy-collector`). |
| `datasources.{postgres,loki,prometheus}` | Names of the Grafana datasources the dashboards resolve to. |
| `postgresDatasource.*` | Read-replica connection + a Secret holding the read-only password (set `create: false` to manage it yourself). |

The dashboard JSON references datasources by the placeholders `${DS_POSTGRES}`, `${DS_LOKI}`,
`${DS_PROMETHEUS}`; each `GrafanaDashboard.spec.datasources` maps those to the configured datasource
names, and the operator resolves them to UIDs at import.

## Dashboards

| File | Forwards | Datasource |
|---|---|---|
| `overview.json` | UI landing KPIs | Postgres |
| `task-runs.json` | runs list + detail (+ Loki drill-down) | Postgres, Loki |
| `repositories.json` | repositories view | Postgres |
| `ingress-dispatcher.json` | webhook + queue/dispatch health | Postgres, Loki |
| `operations.json` | RED metrics | Prometheus |

`operations.json` needs the control plane's `/metrics` scraped into Prometheus (Alloy). The `serve`
and `dispatcher` pods expose `/metrics`; annotate them for Alloy discovery (defaults
`prometheus.io/scrape`, `prometheus.io/port`, `prometheus.io/path` — adjust to your Alloy relabel
config).
