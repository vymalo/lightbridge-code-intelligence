# Dashboard generator

Generates the Lightbridge Grafana dashboards as JSON into the Helm chart at
`deploy/observability/dashboards/`, using the
[grafana-foundation-sdk](https://github.com/grafana/grafana-foundation-sdk). The Grafana Operator
imports them (see `deploy/observability/`).

## Regenerate

```bash
python3 -m venv tools/dashboard-gen/.venv
tools/dashboard-gen/.venv/bin/pip install -r tools/dashboard-gen/requirements.txt
cd tools/dashboard-gen && .venv/bin/python generate.py
```

The committed `deploy/observability/dashboards/*.json` are generated artifacts — **edit the Python,
not the JSON.** CI fails if they are out of date.

## Layout

- `lci_dashboards/common.py` — datasource placeholders (`${DS_POSTGRES}` / `${DS_LOKI}` /
  `${DS_PROMETHEUS}`, substituted by the operator), raw query helpers, grid layout.
- `lci_dashboards/<name>.py` — one module per dashboard, each exposing `dashboard_builder()`.
- `generate.py` — renders every dashboard to deterministic JSON (sorted keys, stable order).
