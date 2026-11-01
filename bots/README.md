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
| [diag-k8s](diag-k8s.json) | `delegation` (and `manual`) | `get`, `describe`, `events`, `logs`, `top`, `diff`, `rollout status`/`history`; says the cause with evidence |

| Setting | Value |
|---|---|
| delegation | `triage` → `diag-k8s`, `maxDepth` 1, `split: even`, `maxChildren` 2, `budgetShare` 0.5 |
| budgets | `triage` 48 model / 48 tool calls, 900 s; a child gets a quarter of that (12 / 12) |
| model | `anthropic/claude-haiku-5.5`, `cost-optimized`, through OpenRouter |
| placement | the node `gx10-ff95`; set `NODE` for another |
| approvals | `autoApprove` for `rollout status` and `rollout history` (AXLR asks a decision for them although the rules allow them); a held call waits `2m`, then is denied |

## The kubectl rules

Every allowed call starts with `--context kind-sre-lab`, then the verb:

| Allowed | Denied anywhere in the arguments |
|---|---|
| `get`, `describe`, `logs`, `events`, `top`, `diff`, `rollout status`, `rollout history` | `delete`, `apply`, `scale`, `edit`, `patch`, `create`, `replace`, `set`, `label`, `annotate`, `exec`, `cordon`, `drain`, `secret`, `secrets` |

The patterns end in `**`: a single `*` stops at `/` (29 Oct). The missions say that `kubectl diff` exit 1 means differences, not an error.

## Register

After `lab/up.sh` (it makes `lab/.secret`):

```bash
bots/register.sh
```

It puts the secret into the `alertmanager` trigger, registers each bot with `axlr-bots bot register --file` (or `bot update` when it exists) and enables it. By hand:

```bash
jq --arg s "$(cat lab/.secret)" '(.triggers[] | select(.type=="alertmanager") | .secret) = $s' bots/triage.json > /tmp/triage.json
axlr-bots bot register --file bots/diag-k8s.json
axlr-bots bot register --file /tmp/triage.json
axlr-bots bot enable diag-k8s
axlr-bots bot enable triage
```

Check: `axlr-bots group get sre --tenant ops` lists both; `axlr-bots bot get triage` shows the trigger `alerts` with `"secret": "<redacted>"`.
