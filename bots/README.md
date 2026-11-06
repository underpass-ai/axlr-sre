# The bots of phase 1

Three bots of the team `sre`, tenant `ops`. `triage` and `diag-k8s`, level `observe`, read the cluster and say the cause; they run on the node [sre-ro](../nodes/sre-ro/README.md) and reach the cluster only through `kube-mcp`, read-only, as the ServiceAccount `axlr-sre-read`. `fix`, level `propose`, proves the change with a server-side `diff` and proposes it with its rollback; it runs on the node [sre-rw](../nodes/sre-rw/README.md) as `axlr-sre-write` and applies nothing. In phase 0 the bots called `kubectl` through `local_exec` under glob rules with the host's admin kubeconfig; that is gone.

```text
Alertmanager ──▶ trigger alerts ──▶ incident + task of triage ──axlr_bots_delegate (wait)──▶ diag-k8s
                                                 ▲                                              │
                                                 └──────────── cause + evidence ◀───────────────┘
                                    Diagnosed ──▶ task of fix (sre-rw): diff, rollback ──▶ proposal
                                    (submitted by sre-judge until the control plane launches the fix phase, design §4)
```

| Bot | Trigger | Does |
|---|---|---|
| [triage](triage.json) | `alertmanager` `alerts` (and `manual`) | reads the alert, delegates to `diag-k8s`, checks its answer, reports `CAUSE` / `EVIDENCE` / `FIX (not applied)` |
| [diag-k8s](diag-k8s.json) | `delegation` (and `manual`) | `get`, `describe`, `events`, `logs`, `rollout_history`; says the cause with evidence |
| [fix](fix.json) | `manual` | reads [fix-runbook.md](fix-runbook.md), runs `diff` of `desired/`, records the live values; reports `CAUSE` / `DIFF` / `PROPOSAL (not applied)` / `ROLLBACK` / `CHECK` |

`triage` and `diag-k8s` read [runbook.md](runbook.md) from their workspace before the first cluster call, `fix` reads [fix-runbook.md](fix-runbook.md) (`register.sh` copies each there as `runbook.md`): the reads each alert needs, as tool calls, and the facts the report must carry. Every message a bot sends that is not a tool call is its whole report: the execution's answer is the last message, and AXLR's Jev final check can add a turn after it. A mission is at most 2000 bytes, which is why the detail lives in the runbook.

| Setting | Value |
|---|---|
| delegation | `triage` → `diag-k8s`, `maxDepth` 1, `split: even`, `maxChildren` 2, `budgetShare` 0.5 |
| budgets | `triage` 64 model / 64 tool calls, 900 s; a child gets a quarter of that (16 / 16). Phase 0 had 48 (12 for a child); an MCP tool costs one more call than `local_exec`, since the model lists the server's tools with `axlr_tools` first, and on 5 November hpa-maxed's child ran out at 12 |
| budgets of `fix` | 24 model / 24 tool calls, 600 s |
| model | `anthropic/claude-haiku-5.5`, `cost-optimized`, through OpenRouter; `fix` too, though design §6 gives it the strong model: on drift the cheap one proposed the expected change (7 / 7 on 6 November) |
| placement | `{"selector": {"role": "sre-ro"}}`: the node sre-ro, dedicated; `fix` `{"role": "sre-rw"}` |
| approvals | none expected: every allowed tool is a read, and `diff` is a dry run that `propose` lets through as an allow-listed MCP tool; a held call waits `2m`, then is denied |

## The tools

| Allowed | What |
|---|---|
| `local_read`, `local_list`, `local_search` | the workspace: `runbook.md` |
| `discover`, `get`, `describe`, `logs`, `events`, `rollout_history` | the `kube` server's reads ([kube-mcp](../tools/kube/README.md)), by their own names: an axlr-bots tool name takes no `/`, and AXLR's policy matches an MCP tool by its own name too. The node's `mcp.json` holds only `kube`, so the names are unambiguous |

`fix` has the same reads but `logs`, and `diff`: the AXLR manifest of sre-rw lets through no other write tool of kube-mcp.

No tool rules: nothing is narrowed by glob any more. kube-mcp refuses Secrets, namespaces outside `KUBE_NAMESPACES` and anything that is not a Kubernetes name before kubectl runs, and RBAC refuses the rest.

## Register

After `lab/up.sh` (it makes `lab/.secret`), [`nodes/sre-ro/install.sh`](../nodes/sre-ro/README.md) and [`nodes/sre-rw/install.sh`](../nodes/sre-rw/README.md):

```bash
bots/register.sh
```

It copies each bot's runbook into its workspace on its node (`~/.local/state/axlr-node-<node>/workspaces/<bot>`, under `AXLR_NODE_STATE`), puts the secret into the `alertmanager` trigger, registers each bot with `axlr-bots bot register --file` (or `bot update` when it exists) and enables it. By hand:

```bash
jq --arg s "$(cat lab/.secret)" '(.triggers[] | select(.type=="alertmanager") | .secret) = $s' bots/triage.json > /tmp/triage.json
axlr-bots bot register --file bots/diag-k8s.json
axlr-bots bot register --file /tmp/triage.json
axlr-bots bot register --file bots/fix.json
axlr-bots bot enable diag-k8s
axlr-bots bot enable triage
axlr-bots bot enable fix
```

Check: `axlr-bots group get sre --tenant ops` lists the three; `axlr-bots bot get triage` shows the trigger `alerts` with `"secret": "<redacted>"`.
