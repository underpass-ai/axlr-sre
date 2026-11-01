# AXLR Bots SRE

A brigade of bots that resolves Kubernetes and cloud incidents on [AXLR Bots](https://github.com/underpass-ai/axlr-bots): it receives the alert, diagnoses with evidence, proposes the fix as a diff, applies it when a person approves, verifies and writes the postmortem. The model proposes; the policy and the person decide; the panel shows everything.

**Status:** phase 0. The design is [docs/design.md](docs/design.md) (Spanish, revision 2.1). The lab, its ten scenarios and the two read-only bots run on a kind cluster; the first run is [below](#phase-0-run-31-october-2026).

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
tools/kube/         one MCP tool per kubectl verb (planned)
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

## License

Apache License 2.0; see [LICENSE](LICENSE). Part of [Underpass AI](https://underpassai.com).
