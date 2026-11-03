# kube-mcp

Kubernetes through `kubectl`, one MCP tool per verb. For a diagnosis bot: what is running, what is failing and why. For a remediation bot, on a node registered with writes: a previewed `apply`, `rollout` and `scale`, each checked against a policy by tool and by namespace, never by glob.

```text
bot ──stdio──▶ kube-mcp ──spawn──▶ kubectl ──▶ API server
                 │                    │
                 │ policy: namespace  │ RBAC: the ServiceAccount of the node
                 │ allowlist, writes  │ decides again, whatever the policy says
                 │ on/off, replica    │
                 │ cap, no secrets,   │
                 │ no blast-radius    │
                 │ kinds              │
```

## Configuration

| Variable | Required | What |
|---|---|---|
| `KUBE_NAMESPACES` | no | comma-separated allowlist; the first is the default of every call. Unset: any namespace, named on every call, and RBAC decides |
| `KUBE_WRITES` | no | `true` publishes the write tools; unset or `false` is read-only. Only on the node whose ServiceAccount may write |
| `KUBE_MAX_REPLICAS` | no | the most `scale` may set, 1 to 1000; `10` by default |
| `KUBE_CONTEXT` | no | a kubeconfig context; the current one by default |
| `KUBECTL` | no | path to the kubectl binary; `kubectl` on the PATH by default |

Credentials are kubectl's own: the kubeconfig (`KUBECONFIG`) or the ServiceAccount token of the pod. This server never reads them and never shows them.

## Tools

Every server:

| Tool | Arguments | Answers |
|---|---|---|
| `doctor` | — | configuration, kubectl version, API server version, `auth can-i list pods` per allowed namespace; with writes, the `diff` program and `auth can-i patch deployments` too; each failure with its action |
| `discover` | `namespace` | per namespace in scope: pods by phase and the ones not ready, deployments short of replicas, how many Warning events |
| `get` | `kind`, `name`, `namespace`, `selector`, `limit` 1–200 (50), `raw` | summaries: name, phase, ready, restarts, reason, replicas; `raw` gives the objects without `managedFields` |
| `describe` | `kind`, `name`, `namespace` | the text of `kubectl describe`, 64 KiB at most |
| `logs` | `pod`, `namespace`, `container`, `tail` 1–1000 (200), `since`, `previous` | the last lines, 256 KiB at most |
| `events` | `namespace`, `object`, `warnings_only` (true), `limit` 1–200 (50) | newest first: type, reason, object, message, count, last seen |
| `rollout_status` | `kind`, `name`, `namespace`, `timeout_seconds` 1–600 (120) | `complete`, or `timed_out` with what is still rolling; an error is an error |
| `rollout_history` | `kind`, `name`, `namespace` | the revisions, for `rollout_undo` |

With `KUBE_WRITES=true`:

| Tool | Arguments | Preview (`confirm` false) | `confirm=true` |
|---|---|---|---|
| `diff` | `manifest`, `namespace` | — | `kubectl diff`: `differences` true or false and the diff. Exit 1 is differences, not an error |
| `apply` | `manifest`, `namespace`, `confirm` | the diff and a `--dry-run=server` | the diff, the dry run and then the apply; a refused dry run stops it |
| `rollout_restart` | `kind`, `name`, `namespace`, `confirm` | the workload as it is now and the command | `kubectl rollout restart` |
| `rollout_undo` | `kind`, `name`, `namespace`, `revision`, `confirm` | the same | `kubectl rollout undo [--to-revision]` |
| `scale` | `kind`, `name`, `replicas`, `namespace`, `confirm` | the current count, the one asked and the cap | `kubectl scale --replicas`, inside `KUBE_MAX_REPLICAS` |

`kind` for `rollout_*` is `deployment`, `statefulset` or `daemonset`; for `scale`, `deployment`, `statefulset` or `replicaset`.

## What is refused before kubectl runs

| Refused | Where |
|---|---|
| a namespace outside `KUBE_NAMESPACES` | every tool |
| `secrets`, however spelled | `get`, `describe` |
| a manifest with a Secret, Namespace, PersistentVolume(Claim), CustomResourceDefinition, Node, Role, ClusterRole, their bindings, NetworkPolicy, an admission webhook or policy | `diff`, `apply` |
| a manifest object whose `metadata.namespace` is not the call's | `diff`, `apply` |
| a `kind: List` | `diff`, `apply`: send the objects as documents |
| any token starting with `-`, any name that is not a Kubernetes name | every tool |
| a write on a read-only server | by name, with the hint to register on the write node |

Also: child kubectl never inherits this process's stdio (the MCP protocol); `KUBECTL_EXTERNAL_DIFF` is dropped from its environment; a run longer than 90 s is killed; every kubectl call carries `--request-timeout=30s`.

## Register it

The read-only node (`sre-ro`) and the write node (`sre-rw`) of the design (§7.2) register the same binary with different variables:

```json
{"command": "/home/<user>/.local/bin/kube-mcp", "env": {"KUBE_NAMESPACES": "shop,payments"}}
```

```json
{"command": "/home/<user>/.local/bin/kube-mcp", "env": {"KUBE_NAMESPACES": "shop,payments", "KUBE_WRITES": "true", "KUBE_MAX_REPLICAS": "20"}}
```

## Layout

```text
src/domain/        tokens (validated), Policy, Manifest (parsed, policed), Invocation (the kubectl arguments), ServerConfig
src/ports/         KubectlRunner: run an Invocation, get exit code, stdout, stderr
src/application/   InspectCluster (reads), ChangeCluster (previewed writes), DiagnoseSetup (doctor), DiscoverScope (discover)
src/adapters/      outbound: the process spawner; inbound: the catalogue and the dispatch
src/bin/           the composition and the stdio server
```

`make -C tools ci-kube` runs this crate's gates.
