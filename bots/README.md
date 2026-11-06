# The bots of phase 1 (read-only)

Two bots of the team `sre`, tenant `ops`, level `observe`: they read the cluster and say the cause; they change nothing. They run on the node [sre-ro](../nodes/sre-ro/README.md) and reach the cluster only through `kube-mcp`, read-only, as the ServiceAccount `axlr-sre-read`. In phase 0 they called `kubectl` through `local_exec` under glob rules with the host's admin kubeconfig; that is gone.

```text
Alertmanager ──▶ trigger alerts ──▶ incident + task of triage ──axlr_bots_delegate (wait)──▶ diag-k8s
                                                 ▲                                              │
                                                 └──────────── cause + evidence ◀───────────────┘
```

| Bot | Trigger | Does |
|---|---|---|
| [triage](triage.json) | `alertmanager` `alerts` (and `manual`) | reads the alert, delegates to `diag-k8s`, checks its answer, reports `CAUSE` / `EVIDENCE` / `FIX (not applied)` |
| [diag-k8s](diag-k8s.json) | `delegation` (and `manual`) | `get`, `describe`, `events`, `logs`, `rollout_history`; says the cause with evidence |

Both read [runbook.md](runbook.md) from their workspace before the first cluster call (`register.sh` copies it there): the reads each alert needs, as tool calls, and the facts the report must carry. Every message a bot sends that is not a tool call is its whole report: the execution's answer is the last message, and AXLR's Jev final check can add a turn after it. A mission is at most 2000 bytes, which is why the detail lives in the runbook.

| Setting | Value |
|---|---|
| delegation | `triage` → `diag-k8s`, `maxDepth` 1, `split: even`, `maxChildren` 2, `budgetShare` 0.5 |
| budgets | `triage` 64 model / 64 tool calls, 900 s; a child gets a quarter of that (16 / 16). Phase 0 had 48 (12 for a child); an MCP tool costs one more call than `local_exec`, since the model lists the server's tools with `axlr_tools` first, and on 5 November hpa-maxed's child ran out at 12 |
| model | `anthropic/claude-haiku-5.5`, `cost-optimized`, through OpenRouter |
| placement | `{"selector": {"role": "sre-ro"}}`: the node sre-ro, dedicated |
| approvals | none expected: every allowed tool is a read; a held call waits `2m`, then is denied |

## The tools

| Allowed | What |
|---|---|
| `local_read`, `local_list`, `local_search` | the workspace: `runbook.md` |
| `discover`, `get`, `describe`, `logs`, `events`, `rollout_history` | the `kube` server's reads ([kube-mcp](../tools/kube/README.md)), by their own names: an axlr-bots tool name takes no `/`, and AXLR's policy matches an MCP tool by its own name too. The node's `mcp.json` holds only `kube`, so the names are unambiguous |

No tool rules: nothing is narrowed by glob any more. kube-mcp refuses Secrets, namespaces outside `KUBE_NAMESPACES` and anything that is not a Kubernetes name before kubectl runs, and RBAC refuses the rest.

## Register

After `lab/up.sh` (it makes `lab/.secret`) and [`nodes/sre-ro/install.sh`](../nodes/sre-ro/README.md):

```bash
bots/register.sh
```

It copies `runbook.md` into each bot's workspace on sre-ro (`AXLR_NODE_WORKSPACES`, default `~/.local/state/axlr-node-sre-ro/workspaces`), puts the secret into the `alertmanager` trigger, registers each bot with `axlr-bots bot register --file` (or `bot update` when it exists) and enables it. By hand:

```bash
jq --arg s "$(cat lab/.secret)" '(.triggers[] | select(.type=="alertmanager") | .secret) = $s' bots/triage.json > /tmp/triage.json
axlr-bots bot register --file bots/diag-k8s.json
axlr-bots bot register --file /tmp/triage.json
axlr-bots bot enable diag-k8s
axlr-bots bot enable triage
```

Check: `axlr-bots group get sre --tenant ops` lists both; `axlr-bots bot get triage` shows the trigger `alerts` with `"secret": "<redacted>"`.
