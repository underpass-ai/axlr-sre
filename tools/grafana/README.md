# grafana-mcp

Grafana, read only, with a service account token. For a diagnosis bot: the log lines Loki holds, the metrics Prometheus holds, the dashboards that say which of them matter and the alerts that are firing. Loki and Prometheus are reached through Grafana's datasource proxy, so one URL and one token reach them all.

```text
bot ──stdio──▶ grafana-mcp ──Bearer token──▶ Grafana ──proxy──▶ Loki
                                               │               Prometheus
                                               │               Alertmanager
                                               └── its own: dashboards, alerts, datasources
```

## Configuration

| Variable | Required | What |
|---|---|---|
| `GRAFANA_URL` | yes | `https://grafana.example.com` or `https://host/grafana`; `http` only for localhost |
| `GRAFANA_TOKEN` | yes | a service account token with the Viewer role; with or without the `Bearer ` prefix; read once at start, never written |
| `GRAFANA_LOKI_DATASOURCE` | no | the uid of the Loki datasource the Loki tools use when a call names none; without it, the only Loki datasource, or the default one among several |
| `GRAFANA_PROMETHEUS_DATASOURCE` | no | the same for Prometheus |

A Viewer may list datasources (checked on Grafana 13.3), so the uids are optional; set them when the listing is denied, and no call needs it then.

## Tools

| Tool | Arguments | Answers |
|---|---|---|
| `doctor` | — | configuration, Grafana's version, whether the token reaches the organisation, the datasources and which Loki and Prometheus the queries will use; each failure with its action |
| `discover` | — | the datasources with their uids and types, the first 20 dashboards, the alert counts by state |
| `loki_query` | `query` (LogQL), `datasource`, `start`, `end`, `limit` 1–1000 (100), `direction` backward/forward | the lines, newest first: time, labels, line; `next.end` (or `next.start`) to page on; 256 KiB and 4 KiB per line at most |
| `loki_labels` | `datasource`, `start`, `end` | the label names in the window |
| `loki_label_values` | `label`, `datasource`, `start`, `end` | the label's values in the window, 500 at most |
| `metrics_query` | `query` (PromQL), `datasource`, `time` | one value per series, 50 series at most |
| `metrics_range` | `query`, `datasource`, `start`, `end` | about a hundred points per series, 50 series at most |
| `dashboards` | `query`, `tag`, `limit` 1–100 (25) | uid, title, folder, tags, url |
| `dashboard` | `uid`, `with_queries` (true) | the variables and the panels, the rows' children included, each with its type, datasource and query expressions; 100 panels at most |
| `alerts` | `datasource` (Grafana's own when omitted), `firing_only` (true) | name, state, severity, namespace, summary, since when; counts by state; 100 at most |

Times are RFC 3339 (`2026-11-03T10:00:00Z`) or how long ago (`15m`, `2h`, `1d`); a window is seven days at most and defaults to the last hour. A `datasource` is a uid, as `discover` lists them.

Paging logs: `loki_query` answers `next.end` when the page was full; call again with that `end` to go further back (or `next.start` forward). Lines that share the boundary nanosecond and were not in the page are lost; a `limit` well above the lines per second avoids it.

## Register it

```json
{"command": "/home/<user>/.local/bin/grafana-mcp", "env": {"GRAFANA_URL": "https://grafana.example.com", "GRAFANA_TOKEN": "...", "GRAFANA_LOKI_DATASOURCE": "loki", "GRAFANA_PROMETHEUS_DATASOURCE": "prometheus"}}
```

## Layout

```text
src/domain/        ServerConfig, TimeWindow, Datasource, Request (validated, segments and pairs), the parsers of each answer
src/ports/         GrafanaReader: the JSON a GET answers
src/application/   Datasources (which uid), QueryLogs, QueryMetrics, InspectGrafana, DiagnoseSetup (doctor), DiscoverScope (discover)
src/adapters/      outbound: the HTTP client; inbound: the catalogue and the dispatch
src/bin/           the composition and the stdio server
```

`make -C tools ci-grafana` runs this crate's gates.
