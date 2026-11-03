# argo-workflows-mcp

Argo Workflows, read only, over Argo Server's HTTP API. For a diagnosis bot: which run failed, what it said, which template or cron it came from.

## Configuration

| Variable | Required | What |
|---|---|---|
| `ARGO_SERVER` | yes | `https://argo.example.com`, `argo.example.com:2746` or, for a lab, `http://localhost:2746`; `http` is refused for any other host |
| `ARGO_TOKEN` | yes | the bearer token of a service account that may `get` and `list` workflows, templates and cron workflows in the namespace; with or without the `Bearer ` prefix; read once at start, never written |
| `ARGO_NAMESPACE` | no | the namespace the tools use when a call names none; `argo` by default |

TLS uses the host's trust store. A token that expires needs the process restarted with a new one; `doctor` says when that has happened.

## Tools

| Tool | Arguments | Answers |
|---|---|---|
| `doctor` | — | three checks: configuration, server version, the namespace readable; each failure with its action |
| `discover` | `namespace` | workflows by phase, template and cron names, one page of 100 each, and whether there is more |
| `list_workflows`, `list_workflow_templates`, `list_cron_workflows` | `namespace`, `limit` 1–100 (25), `continue` | summaries: name, phase, message, times, schedule; the next page's `continue` |
| `get_workflow`, `get_workflow_template`, `get_cron_workflow` | `name`, `namespace` | the whole resource |
| `get_workflow_logs` | `name`, `namespace`, `pod_name`, `tail_lines` 1–200 (100) | the last lines, 256 KiB at most; narrow with `pod_name` past that |

Names and namespaces are validated as Kubernetes names before any request. A `401` or `403` says to check the token and the service account's permissions and does not repeat the server's body.

## Register it

```json
{"command": "/home/<user>/.local/bin/argo-workflows-mcp", "env": {"ARGO_SERVER": "https://argo.example.com", "ARGO_NAMESPACE": "ci", "ARGO_TOKEN": "..."}}
```

## Layout

```text
src/domain/        ArgoQuery (validated), ServerConfig (from the environment), their errors
src/ports/         ArgoReader: the JSON an API GET answers
src/application/   InspectArgo (summaries and details), DiagnoseSetup (doctor), DiscoverScope (discover)
src/adapters/      outbound: the HTTP client; inbound: the catalogue and the dispatch
src/bin/           the composition and the stdio server
```

`make -C tools ci-argo-workflows` runs this crate's gates.
