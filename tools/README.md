# The tools

MCP servers the SRE bots call: one process per system, served over stdio, read-only unless a tool says otherwise. Every server is a Rust crate of this workspace built on [`mcp/`](mcp/), and every one answers `doctor` and `discover` before anything else.

```text
bot (AXLR) ──stdio──▶ <tool>-mcp ──▶ the system's API
                        │
                        ├─ doctor    can I work? what is missing, and what fixes it
                        ├─ discover  what is in scope, bounded
                        └─ the tool's own reads (and, where it says so, its writes)
```

| Crate | Binary | Talks to | Writes |
|---|---|---|---|
| [`mcp/`](mcp/) | — | the MCP client, over `rmcp` | — |
| [`argo-workflows/`](argo-workflows/) | `argo-workflows-mcp` | Argo Server's HTTP API | no |
| [`kube/`](kube/) | `kube-mcp` | `kubectl`, one tool per verb | only with `KUBE_WRITES=true`, previewed, inside a namespace allowlist and a replica cap |
| [`grafana/`](grafana/) | `grafana-mcp` | Grafana's HTTP API, and Loki, Prometheus and Alertmanager through its datasource proxy | no |

`kube/` is the one tool per `kubectl` verb of [design §7.1](../docs/design.md); the rest of what is planned, and in which order, is in [docs/tools-port.md](../docs/tools-port.md).

## Build and register

```bash
make -C tools build                       # target/release/<tool>-mcp
make -C tools install                     # every binary to ~/.local/bin
make -C tools ci                          # fmt, clippy, tests, 80 % of lines per crate, no vendor words
```

A bot's node registers a server in the `mcp.json` AXLR reads, with the variables that server's README names in its environment. No credential goes into a file this repository tracks: a token is read from the environment once, at start.

## What every server keeps to

| Rule | Why |
|---|---|
| `doctor` and `discover` in every catalogue | a model finds out what is wrong, and what is in scope, without guessing |
| stdout is the protocol's | a stray print breaks the client; diagnostics go to stderr |
| no secret in any answer or error | the answers end up in an execution's record |
| a write previews by default and runs on `confirm=true` | the policy and the person decide; the preview path never reaches the write port |
| bounded answers: pages, tails, byte limits | an execution has a budget |
| domain, ports, use cases and adapters apart | the use cases are tested with doubles, without network or credentials |
| 80 % of lines covered per crate, and `make ci` green | the same bar as [axlr-bots' CI](https://github.com/underpass-ai/axlr-bots) |
| nothing of any one installation: no account, region, team, tenant or environment name as a default | these are generic; everything comes from configuration |
