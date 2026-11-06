# The judge

`sre-judge` runs the scenarios against the bots and has Jev, TypeSafe's judgement model, say whether each diagnosis states the facts the scenario expects; for a write-node scenario, whether `fix`'s proposal does. It measures the exit numbers of design §11 and §12: causes (or proposals) correct, duplicates, Open → Diagnosed, cost.

```text
for each scenario                                   Jev, one question per fact
  inject.sh ──▶ Alertmanager ──▶ incident ──▶ Diagnosed ──▶ "does the diagnosis state <fact>?" ──▶ stated / missing / contradicted
                                                   │
  clear.sh ◀── verdict, times, cost, duplicates ◀──┘           ──▶ judge/results/<scenario>.json ──▶ the tables
```

## Run it

```bash
judge/run.sh                        # every scenario the read node can diagnose; image-pull left firing an hour for the resends
judge/run.sh --only oom,drift       # some scenarios, no soak; drift only this way
judge/run.sh --soak-for 0           # every scenario, without the hour
go run ./cmd/sre-judge report       # the tables again, from judge/results
go run ./cmd/sre-judge score --expected scenarios/oom/expected.md --incident <id>   # one incident, offline
```

The Jev key is `TYPESAFE_API_KEY`; `run.sh` reads it from the environment or from `~/.config/axlr-bots/env` (the node's file) and never prints it. The control plane is read over its socket, as the `axlr-bots` CLI does; the judge's one write is a task for `fix`. Each node's scenarios are judged against that node's goals (`domain/goals.go`), in a table of their own when the results hold both, and exit 1 says one node's part does not clear them:

| Goals | Scenarios | Bar |
|---|---|---|
| `ReadNode` (decided 5 November 2026) | the nine sre-ro diagnoses | all fire, at least eight causes correct, 0 duplicates, under $1 |
| `WriteNode` (set 6 November 2026) | drift, sre-rw's only one so far | it fires, and `fix`'s proposal states every fact |

Phase 0's ten-scenario bar is kept as `Phase0`.

| Flag | Default | |
|---|---|---|
| `--timeout` | `15m` | how long an incident may stay `Open` (pvc-full took 14 min on 31 October) |
| `--poll` | `5s` | how often the incidents are read |
| `--soak`, `--soak-for` | `image-pull`, `60m` from `run.sh` | the scenario left firing after its diagnosis, and for how long |
| `--out` | `judge/results` | one JSON per scenario as it lands; a run that breaks resumes with `--only` |
| `--model` | `jev-1.13.0` | the Jev version, pinned, the one AXLR's `axlr-v2` judge policy and axlr-bots' Jev runtime use |
| `--fix-bot`, `--fix-timeout` | `fix`, `10m` | the bot that proposes a write-node scenario's fix, and how long its task may take |

A scenario whose directory holds `needs-write-node` is left out of the default run; `--only` runs it. Its incident is still triage's, on the read node; once it is `Diagnosed` the judge submits a task to `fix` with triage's diagnosis (key `sre-judge:<incident>:<cycle>:fix`), waits for it to end, and Jev judges `fix`'s answer instead of the diagnosis. That is the control plane's job in design §4 (Diagnosed → Proposed), which axlr-bots does not do yet; the judge stands in for it. The fix task's cost is added to the scenario's. Phase 0's verdicts stay in `judge/results/phase-0/`; `report` reads only the files directly under `--out`.

`node-notready` runs last: it stops a worker's kubelet, and the faults before it schedule onto that worker. After every `clear.sh` the judge waits for every node `Ready`.

## How it judges

One question to Jev per row of `expected.md`: the state is the fact and the diagnosis, the options `stated`, `missing`, `contradicted`. Jev's own choice is recorded with its probability; no threshold is applied. The rule over the cause facts (every row but `fix (not applied)`):

| Verdicts | Cause |
|---|---|
| every cause fact `stated` | **correct** |
| some stated, none contradicted | **partly** |
| any contradicted, or none stated, or no diagnosis (`Escalated`, timeout), or a write-node scenario's fix task ended without an answer | **wrong** |

The fix row is shown, not counted. A write-node scenario's `expected.md` names its proposal and rollback rows otherwise (`proposal (not applied)`, `rollback`), so they count. The tables list every fact's verdict, so a person can audit Jev. The question says a fact is stated only when every part of it is: a compound row (`load … from the Deployment load`) was judged by its gist until it was split in two, which is why `expected.md` lists one fact per row.

## What it measures

| Column | From |
|---|---|
| trigger answers | the bot's events `incident.opened`, `incident.reopened`, `incident.repeated` since the injection |
| Inject → settled | the judge's clock, polled every `--poll` |
| Open → settled | the events' timestamps: the cycle's opening to its `incident.diagnosed` or `incident.escalated` |
| cost | the incident's `costUsd` plus every delegated child's executions (`GET /v1/tasks/{id}/executions`), because the incident counts only the triage task (PR 3's finding) |
| resends | the incident's `repeats` |
| duplicates | incidents of the same alert opened since the injection, beyond the first |
| other alerts | incidents of other alerts opened since the injection: side effects, not duplicates |
| cost of judging | Jev's input tokens at $0.042 per million |

A reopening within 30 minutes (same incident, next cycle) counts as the scenario's incident. Nothing is cleared between scenarios on the bots' side: each incident is a fresh task, and the bots keep no state across tasks in phase 0.

## Layout

```text
cmd/sre-judge/           run, score, report
judge/domain/            facts, questions, verdicts, the score rule, the result and the run's goals
judge/application/       the use cases over the ports Lab, ControlPlane, Judge, Clock
judge/adapters/jev/      TypeSafe's API, as axlr-bots' adapters/jev calls it
judge/adapters/controlplane/   the operator API over the socket
judge/adapters/shell/    inject.sh, clear.sh, expected.md, kubectl wait
judge/adapters/markdown/ the tables
judge/adapters/results/  one JSON per scenario
judge/run.sh             the judged run
```

`go test -race ./...`; every package at 80 % or more (CI checks it).
