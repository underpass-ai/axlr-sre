# The bots of phase 0

Two bots of the team `sre`, tenant `ops`, level `observe`: they read the cluster and say the cause; they change nothing.

```text
Alertmanager ──▶ trigger alerts ──▶ incident + task of triage ──axlr_bots_delegate (wait)──▶ diag-k8s
                                                 ▲                                              │
                                                 └──────────── cause + evidence ◀───────────────┘
```

| Bot | Trigger | Does |
|---|---|---|
| [triage](triage.json) | `alertmanager` `alerts` (and `manual`) | reads the alert, delegates to `diag-k8s`, checks its answer, reports `CAUSE` / `EVIDENCE` / `FIX (not applied)` |
| [diag-k8s](diag-k8s.json) | `delegation` (and `manual`) | `get`, `describe`, `events`, `logs`, `top`, `diff`; says the cause with evidence |

Both read [runbook.md](runbook.md) from their workspace before the first `kubectl` call (`register.sh` copies it there): the reads each alert needs and the facts the report must carry. It closes the three gaps of the 1 November run (crashloop's *what changed* from the Deployment's ReplicaSets, pvc-full's PV and StorageClass, drift's `kubectl diff -f desired/` as it is), and `triage` keeps `diag-k8s`'s facts instead of summarising them. Each report ends with a line on memory, and `rollout` is denied: AXLR asks a decision for `rollout`, and in the resumed run its Jev final check and memory reminder add turns whose last sentence replaced the report (crashloop, 1 November, twice). A mission is at most 2000 bytes, which is why the detail lives in the runbook.

| Setting | Value |
|---|---|
| delegation | `triage` → `diag-k8s`, `maxDepth` 1, `split: even`, `maxChildren` 2, `budgetShare` 0.5 |
| budgets | `triage` 48 model / 48 tool calls, 900 s; a child gets a quarter of that (12 / 12) |
| model | `anthropic/claude-haiku-5.5`, `cost-optimized`, through OpenRouter |
| placement | the node `gx10-ff95`; set `NODE` for another |
| approvals | none expected: `rollout` is denied (below); a held call waits `2m`, then is denied |

## The kubectl rules

Every allowed call starts with `--context kind-sre-lab`, then the verb:

| Allowed | Denied anywhere in the arguments |
|---|---|
| `get`, `describe`, `logs`, `events`, `top`, `diff` | `rollout`, `delete`, `apply`, `scale`, `edit`, `patch`, `create`, `replace`, `set`, `label`, `annotate`, `exec`, `cordon`, `drain`, `secret`, `secrets`, and `-f -` (a manifest on stdin: `diff` reads the `desired/` files as they are) |

The patterns end in `**`: a single `*` stops at `/` (29 Oct). The missions say that `kubectl diff` exit 1 means differences, not an error.

## Register

After `lab/up.sh` (it makes `lab/.secret`):

```bash
bots/register.sh
```

It copies `runbook.md` into each bot's workspace (`AXLR_NODE_WORKSPACES`, default `~/.local/state/axlr-node/workspaces`), puts the secret into the `alertmanager` trigger, registers each bot with `axlr-bots bot register --file` (or `bot update` when it exists) and enables it. By hand:

```bash
jq --arg s "$(cat lab/.secret)" '(.triggers[] | select(.type=="alertmanager") | .secret) = $s' bots/triage.json > /tmp/triage.json
axlr-bots bot register --file bots/diag-k8s.json
axlr-bots bot register --file /tmp/triage.json
axlr-bots bot enable diag-k8s
axlr-bots bot enable triage
```

Check: `axlr-bots group get sre --tenant ops` lists both; `axlr-bots bot get triage` shows the trigger `alerts` with `"secret": "<redacted>"`.
