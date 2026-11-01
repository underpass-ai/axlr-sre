# The lab

A kind cluster `sre-lab` on one host, with Prometheus and Alertmanager in it. Alertmanager posts the lab's alerts to the AXLR Bots control plane on the host, to the `alertmanager` trigger `alerts` of the bot `triage`.

```text
kind sre-lab                                              host
┌──────────────────────────────────────────────┐
│ scenario ──▶ kube-state-metrics, kubelet,     │
│              cert-manager metrics             │
│                   │                           │
│              Prometheus (rules.yaml, for: 1m) │
│                   │                           │
│              Alertmanager ───────────────────────▶ https://<kind gateway>:7879/v1/triggers/triage/alerts
└──────────────────────────────────────────────┘     X-Axlr-Secret, fleet CA
                                                     │
                                                     ▼
                                     control plane: incident ──▶ triage ──▶ diag-k8s
```

| Piece | What it is |
|---|---|
| [kind.yaml](kind.yaml) | one control plane, two workers: the monitoring stack on `sre-lab-worker`, node and volume faults on `sre-lab-worker2`. Workers need `fs.inotify.max_user_instances` above 128 on the host (the file says how) |
| [kube-prometheus-stack.yaml](kube-prometheus-stack.yaml) | the chart's values: no default rules, no Grafana, the receiver `axlr-bots` |
| [rules.yaml](rules.yaml) | ten lab alerts, `for: 1m`, label `lab="true"`, built to exclude each other |
| [up.sh](up.sh), [down.sh](down.sh) | bring the lab up or delete it |

## Before

The control plane must serve its operator API where the kind network reaches it, and its certificate must name that address. A user install on the host:

```bash
axlr-bots install --listen 0.0.0.0:7878 --api-listen 0.0.0.0:7879
systemctl --user restart axlr-bots.service axlr-node.service
```

The certificate names every address of the host, the kind gateway (`docker network inspect kind`) included. The node needs the model key and `TYPESAFE_API_KEY` in `~/.config/axlr-bots/env`.

## Up

```bash
lab/up.sh
```

It creates the cluster, installs metrics-server, kube-prometheus-stack and cert-manager with Helm, applies the rules and makes `lab/.secret`, the trigger's secret (never committed). Its last line says what the control plane answered without the secret: `403` proves the address, the TLS and the route.

| Variable | Default |
|---|---|
| `CP_HOST` | the kind network's gateway |
| `CP_PORT` | `7879` |
| `AXLR_CA` | `~/.local/state/axlr-bots/ca/ca.crt` |

Then register the bots ([bots/README.md](../bots/README.md)) and inject a scenario ([scenarios](../scenarios)).

## Alertmanager

| Setting | Value | Why |
|---|---|---|
| `group_by` | `[alertname, namespace]` | the group key does not move when a pod is recreated |
| `group_wait` | `30s` | |
| `group_interval` | `2m` | |
| `repeat_interval` | `2m` | resends the open groups often, to prove they stay one incident |
| `send_resolved` | `true` | a resolved group closes the incident |
| route | `lab="true"` to `axlr-bots`, the rest to `null` | |

Watch it:

```bash
kubectl --context kind-sre-lab -n monitoring port-forward svc/kps-alertmanager 9093
kubectl --context kind-sre-lab -n monitoring port-forward svc/kps-prometheus 9090
```

## Down

```bash
lab/down.sh
```
