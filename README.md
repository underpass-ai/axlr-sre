# AXLR Bots SRE

A brigade of bots that resolves Kubernetes and cloud incidents on [AXLR Bots](https://github.com/underpass-ai/axlr-bots): it receives the alert, diagnoses with evidence, proposes the fix as a diff, applies it when a person approves, verifies and writes the postmortem. The model proposes; the policy and the person decide; the panel shows everything.

**Status:** phase 0. The design is [docs/design.md](docs/design.md) (Spanish, revision 2.1). The lab, its ten scenarios and the two read-only bots run on a kind cluster; the first run is [below](#phase-0-run-31-october-2026), then the judged runs. **Phase 0 passed on 1 November 2026**: 10 / 10 correct under Jev's strict rule, 0 duplicates ([the run](#judged-run-1-november-2026-after-the-bot-fixes)).

```text
Alertmanager ──webhook──▶ axlr-bots control plane ──▶ Incident ──▶ triage ──▶ diag-k8s / diag-cloud ──▶ fix (approval) ──▶ verify ──▶ scribe
```

## What lives where

| Piece | Repo |
|---|---|
| Incident aggregate, `alertmanager` trigger, approvers, per-incident budget | [axlr-bots](https://github.com/underpass-ai/axlr-bots) (the control plane) |
| the bots' definitions, the runbooks as signed skills, the `kube` tool server, the kind lab with its scenarios, the judge, the Helm chart | this repo |

## Layout

```text
docs/design.md      the design and its decisions
lab/                kind + kube-prometheus-stack, the lab's alert rules      (phase 0)
scenarios/          ten faults: inject.sh, clear.sh, expected.md each        (phase 0)
bots/               triage, diag-k8s                                         (phase 0)
judge/              sre-judge: runs the scenarios, Jev judges the diagnoses          (phase 0)
runbooks/           crashloop, oom, image-pull, … as catalog skills (planned)
tools/              the MCP servers the bots call: mcp/ (the server), argo-workflows/ and kube/; grafana/ next (docs/tools-port.md)
deploy/             Helm: two node pods (read-only, write) (planned)
```

## Run it

```bash
lab/up.sh                                   # kind sre-lab, Prometheus, Alertmanager → the control plane
bots/register.sh                            # triage and diag-k8s, enabled
scenarios/oom/inject.sh                     # wait about two minutes for the alert
axlr-bots incident list
axlr-bots incident get <incident-id>        # status, diagnosis, cost
scenarios/oom/clear.sh                      # the resolved group closes the incident
judge/run.sh                                # every scenario, judged by Jev; see judge/README.md
lab/down.sh
```

The control plane and the node must be built from axlr-bots with the Incident aggregate and the `alertmanager` trigger ([#78](https://github.com/underpass-ai/axlr-bots/pull/78), [#80](https://github.com/underpass-ai/axlr-bots/pull/80)). The operator API must listen where the kind network reaches it ([lab/README.md](lab/README.md)).

## Phase 0 run, 31 October 2026

On gx10, with the control plane and the node built from axlr-bots `alertmanager-trigger` (c34e779) and installed user-level. The cluster was kind `sre-lab`: one control plane, two workers, kube-prometheus-stack 92.17.0 (Alertmanager 0.34.2), cert-manager v1.21.3 and metrics-server 3.14.0. The model was `anthropic/claude-haiku-5.5` through OpenRouter for both bots. Each scenario was injected, followed until its incident left `Open`, then cleared; `image-pull` stayed firing for an hour.

| Measure | Result | Phase 0 goal |
|---|---|---|
| scenarios that fired their alert and opened one incident | 10 / 10 | 10 / 10 |
| diagnoses that name the cause (my judgement against `expected.md`; the Jev judge is PR 4) | 9 / 10, and 1 partly | 8 / 10 |
| duplicate incidents | 0 | 0 |
| Open → Diagnosed | 30–45 s (one 486 s, below) | < 3 min |
| cost per incident, triage and its diag-k8s child | $0.005–$0.013 | < $0.10 |
| cost of the whole run | $0.075 | < $1 |

### Per scenario

Times are seconds from `inject.sh` to the incident leaving `Open` (`Diagnosed`), polled every 5 s. Actions are what the trigger answered Alertmanager.

| Scenario | Alert that fired | Actions | Diagnosis, first line (shortened) | Inject → Diagnosed | Open → Diagnosed | Cost | Cause matches |
|---|---|---|---|---|---|---|---|
| crashloop | `LabPodCrashLooping` in shop | opened, resolved; reopened and resolved once more (see pvc-full) | checkout exits 1 because `/etc/checkout/config.yaml` is missing; the new ReplicaSet brought it | 295 | 30 | $0.0125 (2 cycles) | yes |
| oom | `LabPodOOMKilled` in shop | opened, repeated ×2, resolved | `cart` is OOMKilled: it allocates 60 MiB under a 32Mi limit | 160 | 40 | $0.0080 | yes |
| image-pull | `LabImagePullBackOff` in shop | opened, repeated ×16, resolved | `nginx:1.99` does not exist in the registry (`NotFound`) | — (the first sends were refused, below; the alert fired at +120 s) | 30 | $0.0050 | yes |
| missing-secret | `LabContainerConfigError` in shop | opened, resolved | `DB_PASSWORD` references Secret `db`, which does not exist in `shop` | 135 | 30 | $0.0059 | yes |
| pvc-full | `LabVolumeFull` in shop | opened, repeated ×2, resolved | `uploads` writes 200 MiB with `dd` into the 64Mi PVC `uploads-data` (100 %) | 841 | 486 | $0.0072 | yes |
| node-notready | `LabNodeNotReady` | opened, resolved | `sre-lab-worker2`: kubelet stopped posting node status; Ready Unknown, unreachable taints | 191 | 40 | $0.0077 | yes |
| netpol | `LabPodNotReady` in shop | opened, resolved | `payments` probe `nslookup` times out: NetworkPolicy `deny-all` denies all egress, DNS included | 165 | 35 | $0.0069 | yes |
| hpa-maxed | `LabHPAMaxedOut` in shop | opened, resolved | HPA `search` pinned at 3: CPU 180 % of a 10m request against a 50 % target | 195 | 35 | $0.0071 | partly: right state and numbers, but it misses the `load` Deployment and proposes a bigger request |
| cert-expiry | `LabCertificateExpiringSoon` in shop | opened, resolved | `shop-tls` expires in about an hour: `duration: 1h`, issuer `lab-selfsigned` | 155 | 45 | $0.0067 | yes |
| drift | `LabDeploymentDrift` in drift-lab | opened, resolved | `web`: replicas 3 vs 2, `nginx:1.26-alpine` vs `1.27-alpine`; `diff` exit 1 read as differences | 140 | 40 | $0.0082 | yes |

### An hour of resends

`image-pull` fired from 21:45:24 to 22:53 UTC. Alertmanager resent its group 16 times in the 64 minutes to 22:49:24: once every 4 minutes, 15 an hour. The trigger answered `repeated` to every resend, started no task, and `incident list` held one incident for the group, with `repeats: 16`. Across the run the trigger accepted 42 deliveries for 10 groups, and the control plane holds 10 incidents.

### What did not fit

| Finding | Effect | What we did |
|---|---|---|
| The host's `fs.inotify.max_user_instances` was 128, and gx10 is also a Kubernetes node beside the `drift-lab` cluster | the kind workers' kubelets, and then kube-proxy on a lone node, failed with `too many open files` | `sudo sysctl -w fs.inotify.max_user_instances=1024`; [lab/kind.yaml](lab/kind.yaml) says so |
| prometheus-operator re-renders Alertmanager's config and writes `http_headers` `secrets: [...]` as the literal `<secret>` | every delivery for the first 10 minutes was refused with `403` | the secret is a mounted file: `files: [/etc/alertmanager/secrets/axlr-trigger/secret]`. axlr-bots' triggers runbook shows `secrets:` |
| AXLR asks a decision for `kubectl rollout status` and `rollout history` although the bots' rules allow them | in pvc-full, both diag-k8s children stopped on `rollout history`, nobody decided, and they ended `Expired` at their deadline; triage diagnosed alone after 486 s | `approvals.autoApprove` names both calls (drift's child then read its rollout history, `decidedBy: policy:autoApprove`), and a held call waits `2m`, not `15m` |
| `repeat_interval: 2m` equal to `group_interval: 2m` | a resend every 4 minutes, not 2: the repeat is due a moment after each flush and waits for the next one | counted as it is: 15 resends an hour |
| The pvc-full volume is a tmpfs, and its pages count against the writer's memory limit | the first `uploads` container was killed at start (`StartError`, not `OOMKilled`), so `LabPodCrashLooping` fired too and reopened the crashloop incident (cycle 2) for the same namespace | the writer's limit is 256Mi; the next run raises only `LabVolumeFull` |
| kind's local-path volumes report no `kubelet_volume_stats_*` and are not bounded | a full PVC could not alert, and `dd` would fill the host disk | a local PersistentVolume on a 64 MiB tmpfs inside the node |
| The incident's `costUsd` counts only the triage task's executions | a child's cost is not in the incident | the table adds the diag-k8s children by hand; per-incident cost in axlr-bots is open |
| The OOM and crash-loop rules look back 10 and 3 minutes, so a deleted pod keeps them firing | `resolved` reached the control plane up to 561 s after a clear | open: shorter windows cost exclusivity between the rules |
| Prometheus cannot read the desired manifests | the drift rule carries the desired state (2 replicas, `nginx:1.27-alpine`) | the rule says so; the bots still diff against `desired/` |
| A cycle that ends after its group resolved changes nothing | crashloop's cycle 2 completed with "Unconfirmed: the pod no longer exists"; the incident stayed `Resolved` with cycle 1's diagnosis | open |
| The objective renders labels as a Go map (`map[alertname:… namespace:shop …]`) | readable; both bots found every object from it | kept |

## Judged run, 1 November 2026

The same cluster, bots and model as on 31 October, driven by `judge/run.sh` (PR 4): every scenario injected, followed until its incident left `Open`, scored by Jev `jev-1.13.0` against its `expected.md`, then cleared; `image-pull` held for an hour of resends; `node-notready` last. The run took 76 minutes (00:06–01:23 UTC). The per-fact tables are in [judge/results/](judge/results/) and come out of `go run ./cmd/sre-judge report`.

| Measure | Result | Phase 0 goal |
|---|---|---|
| scenarios that fired their alert and opened one incident | 10 / 10 | 10 / 10 |
| diagnoses Jev finds stated in full, every expected fact | 6 / 10, and 4 partly | 8 / 10 |
| diagnoses that name the cause (my judgement, as on 31 October) | 10 / 10 | 8 / 10 |
| duplicate incidents, after 15 resends of `image-pull` in 60 minutes | 0 | 0 |
| Open → Diagnosed | 25–61 s | < 3 min |
| cost of the whole run, diag-k8s children included | $0.071 | < $1 |
| cost of judging, 58 Jev questions | $0.0020 | |

The judge's rule is stricter than the goal's wording: a diagnosis is *correct* only when every cause fact of `expected.md` is stated, *partly* when some are missing and none contradicted. Under that rule the run does not clear phase 0 (`sre-judge` exits 1); under "names the cause" it does. **Decided on 1 November: the strict rule counts.** Phase 0 is not passed; the bots improve, not the rule (what to improve is in the table below). The bots improved the same day and the [run after the fixes](#judged-run-1-november-2026-after-the-bot-fixes) passed.

| Scenario | Open → Diagnosed | Cost | Facts stated | Fix proposed | Jev | What is missing |
|---|---|---|---|---|---|---|
| crashloop | 31 s | $0.0059 | 5 / 6 | no | partly | *what changed*: revision 2 changed the command; the bot blames the missing config volume, never the rollout |
| oom | 39 s | $0.0066 | 5 / 5 | yes | correct | |
| image-pull | 25 s | $0.0055 | 5 / 5 | yes | correct | |
| missing-secret | 28 s | $0.0057 | 5 / 5 | no | correct | |
| pvc-full | 32 s | $0.0068 | 4 / 5 | yes | partly | *volume*: PV `lab-uploads`, StorageClass `lab-local`, so no expansion |
| node-notready | 39 s | $0.0082 | 3 / 4 | no | partly | *side effect*: Jev says the `unreachable` taint is missing (p 0.63); the diagnosis names it and the pods' `NodeNotReady` warnings. Jev's one miss |
| netpol | 30 s | $0.0067 | 4 / 4 | yes | correct | |
| hpa-maxed | 44 s | $0.0068 | 5 / 5 | no | correct | this time the bot found the `load` Deployment |
| cert-expiry | 31 s | $0.0055 | 4 / 4 | yes | correct | |
| drift | 61 s | $0.0134 | 4 / 5 | yes | partly | *evidence*: `kubectl diff` exit 1; the bot's `diff` exited 2 (it fed a manifest without `spec.selector`) and it compared against `last-applied` instead |

"Fix proposed" is Jev's verdict on the `fix (not applied)` row; it is shown, not counted. Three bots wrote a fix but not the one expected (crashloop: mount the config instead of rolling back; node-notready: restart the kubelet, no cordon).

### Jev against a person

| Diagnoses | Person | Jev |
|---|---|---|
| 31 October's ten, scored offline with `sre-judge score` (calibration) | 9 correct, 1 partly | 8 correct, 2 partly: crashloop's *what changed* (p 0.75) and hpa-maxed's *source*, where Jev was right and the person had let it pass |
| 1 November's ten, live | 10 name the cause | 6 correct, 4 partly: three real gaps and one miss (node-notready) |

### What did not fit

| Finding | Effect | What we did |
|---|---|---|
| Jev judges a compound fact by its gist | hpa-maxed's `load` row passed while the Deployment `load` was never named | one fact per row in `expected.md`; the question says *every part* |
| Jev missed a stated fact once in 58 questions (node-notready's taint, p 0.63); that `side effect` row is still compound (the taint and the pods) | one *partly* that a person would mark *correct* | kept as Jev scored it, the row not split after the fact; the per-fact tables let a person audit it |
| The diag-k8s bot strips `spec.selector` before `kubectl diff`, so `diff` exits 2 | drift was diagnosed from the `last-applied` annotation, not from the diff | open: the bot's rules for `diff`, or a `kube` tool that diffs properly |
| `image-pull` sees the other scenarios' alerts while it is held | its result lists five "other alerts" that are the run's own | expected for a soak that runs after the other scenarios; the judge counts duplicates by group key, so they do not count |
| `incident.costUsd` still omits children | | the judge sums the children's executions itself (`incident_cost.go`) |

## Judged run, 1 November 2026, after the bot fixes

**Phase 0 passed:** 10 / 10 correct under the strict rule, 0 duplicates, `sre-judge` exit 0. Same cluster, model (`anthropic/claude-haiku-5.5`), judge (`jev-1.13.0`) and `expected.md` as the run above; only `bots/` changed (branch `bots-gaps`). `judge/run.sh`, 10:50–12:07 UTC, `image-pull` held an hour for the resends, `node-notready` last; the cluster ended clean (`shop` empty, three nodes `Ready`). The tables come out of `go run ./cmd/sre-judge report`; the per-fact verdicts are in [judge/results/](judge/results/).

| Measure | Result | Phase 0 goal |
|---|---|---|
| scenarios that fired their alert and opened one incident | 10 / 10 | 10 / 10 |
| causes Jev found stated in full (partly) | 10 / 10 (0) | 8 / 10 |
| duplicate incidents | 0 | 0 |
| Open → Diagnosed | 33 s–53 s | < 3m0s |
| cost of the whole run, children included | $0.093 | < $1 |
| cost of judging (Jev) | $0.0026 | |
| passes | true | |

| Scenario | Alert | Trigger answers | Inject → settled | Open → settled | Cost | Resends | Duplicates | Other alerts | Facts stated | Fix proposed | Cause |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cert-expiry | `LabCertificateExpiringSoon` | opened | 170 s | 46 s | $0.0104 | 0 | 0 | — | 4 / 4 | no | **correct** |
| crashloop | `LabPodCrashLooping` | opened | 291 s | 39 s | $0.0091 | 0 | 0 | — | 6 / 6 | yes | **correct** |
| drift | `LabDeploymentDrift` | opened | 161 s | 53 s | $0.0091 | 0 | 0 | — | 5 / 5 | yes | **correct** |
| hpa-maxed | `LabHPAMaxedOut` | opened | 205 s | 47 s | $0.0104 | 0 | 0 | — | 5 / 5 | no | **correct** |
| image-pull | `LabImagePullBackOff` | opened, repeated ×15 | 145 s | 39 s | $0.0091 | 15 | 0 | LabPodOOMKilled, LabNodeNotReady, LabVolumeFull, LabPodNotReady, LabContainerConfigError | 5 / 5 | yes | **correct** |
| missing-secret | `LabContainerConfigError` | opened | 146 s | 38 s | $0.0085 | 0 | 0 | — | 5 / 5 | yes | **correct** |
| netpol | `LabPodNotReady` | opened | 176 s | 33 s | $0.0077 | 0 | 0 | — | 4 / 4 | yes | **correct** |
| node-notready | `LabNodeNotReady` | opened | 200 s | 45 s | $0.0104 | 0 | 0 | — | 4 / 4 | no | **correct** |
| oom | `LabPodOOMKilled` | opened | 175 s | 45 s | $0.0095 | 0 | 0 | — | 5 / 5 | yes | **correct** |
| pvc-full | `LabVolumeFull` | opened | 170 s | 38 s | $0.0085 | 0 | 0 | — | 5 / 5 | yes | **correct** |

| Scenario | Jev, first 1 November run | Jev, this run |
|---|---|---|
| crashloop | partly: *what changed* missing | **correct** |
| pvc-full | partly: *volume* missing | **correct** |
| drift | partly: *evidence* missing | **correct** |
| node-notready | partly (Jev's miss) | **correct**, nothing changed for it |
| the other six | correct | correct |

### What changed in the bots

| Change | Why |
|---|---|
| [bots/runbook.md](bots/runbook.md), copied by `register.sh` into each bot's workspace and read first | a mission is at most 2000 bytes; the runbook says which reads each alert needs and which facts the report carries |
| crashloop: compare the Deployment's ReplicaSets (revision, change-cause, command, image) | the bot never looked at what changed; the ReplicaSets show revision 2 `release v2` and the command it changed |
| pvc-full: `describe pv` and `describe storageclass` | the report now names PV `lab-uploads`, StorageClass `lab-local`, `kubernetes.io/no-provisioner` and that it cannot expand |
| drift: `kubectl diff -f desired/` as it is; `-f -` denied | the bot had stripped `spec.selector` and fed the manifest on stdin (exit 2); now it reports exit 1 and the two fields |
| `triage` keeps `diag-k8s`'s facts with their commands | `diag-k8s` had named `lab-uploads` on 1 November and `triage`'s summary dropped it |
| `kubectl rollout` denied, `autoApprove` removed | AXLR asks a decision for `rollout` although the rules allow it; the run then resumes, and the resumed run's extra turns replaced the report |
| every message that is not a tool call is the whole report | the execution's answer is the last message; AXLR's Jev final check (`[AXLR · Jev]`) and memory reminder (`[AXLR · memory]`) add a turn after the report, and a one-sentence reply to it became the diagnosis |

### What did not fit

| Finding | Effect | What we did |
|---|---|---|
| A DNS failure reaching an unrelated MCP server (`learn.microsoft.com`) failed a `triage` task at startup | the incident went `Escalated` with no diagnosis | rerun; a bot should not depend on MCP servers it does not use (axlr-bots / AXLR follow-up) |
| An `Escalated` incident stays open for its group key | the next crashloop alerts were counted as repeats of it and no incident opened; the judge timed out | resolved it by hand; `Escalated` should not absorb a new firing (axlr-bots follow-up) |
| crashloop lost its report three times to a console turn (memory reminder, then Jev's final check, the last without any `rollout`) | 2/6, 0/6 and 1/6 facts on the reruns | the mission rule above; the runtime fix is [underpass-ai/AXLR#189](https://github.com/underpass-ai/AXLR/pull/189) |
| OpenRouter ran out of credit (HTTP 402) during the first full rerun | ten incidents escalated in 4 s, $0; that run is void and not recorded | topped up, run again |
| A rerun within 30 minutes reopens the last incident of the same alert | the crashloop reruns show `reopened` | the full run started 28 minutes after the last resolve, so every scenario opened its own |

## License

Apache License 2.0; see [LICENSE](LICENSE). Part of [Underpass AI](https://underpassai.com).
