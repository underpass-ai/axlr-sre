# The node sre-ro

The read-only node of the design (§7.2): a second `axlr-node` on the host, dedicated to the bots whose placement selects `role=sre-ro`. Its runs reach the cluster only through `kube-mcp`, read-only, with the token of the ServiceAccount `axlr-sre-read`. Two layers decide each call: kube-mcp's policy (namespace allowlist, no Secrets, no write tools) and then RBAC (the ClusterRole in [rbac.yaml](rbac.yaml)), whatever the policy says.

```text
axlr-bots ──socket──▶ axlr-node --node-id sre-ro --dedicated --bus direct
                         │  --axlr-mcp-config ~/.config/axlr-sre/sre-ro/mcp.json
                         ▼
                      axlr-tui (triage, diag-k8s) ──stdio──▶ kube-mcp ──▶ kubectl ──▶ API server
                                                    KUBE_NAMESPACES=shop,drift-lab    token of axlr-sre-read
```

| What | Where |
|---|---|
| ServiceAccount, ClusterRole, binding | [rbac.yaml](rbac.yaml), namespace `axlr-sre`: get, list and watch on what a diagnosis reads; nothing on Secrets |
| kubeconfig with the token | `~/.config/axlr-sre/sre-ro/kubeconfig`, 0600 |
| kube-mcp's AXLR manifest | `~/.config/axlr-sre/sre-ro/kube.json`, plugin id `kube`, the eight read tools |
| the runs' `mcp.json` | `~/.config/axlr-sre/sre-ro/mcp.json`: kube only, so neither the operator's servers nor KMP join the bots' runs |
| state, traces, workspaces | `~/.local/state/axlr-node-sre-ro/` |
| unit | `~/.config/systemd/user/axlr-node-sre-ro.service` |

## Install

After `lab/up.sh` and `make -C tools install`:

```bash
nodes/sre-ro/install.sh
```

It applies `rbac.yaml` with the admin context, mints a token (`TOKEN_TTL`, 720 h by default), writes the files above, refuses to go on if the identity can read Secrets or cannot list pods, and starts the unit. The steps live in [../install-node.sh](../install-node.sh), shared with the write node [sre-rw](../sre-rw/README.md); `SRE_NAMESPACES` is `shop,drift-lab` by default. Run it again to renew the token. Then `bots/register.sh` copies the runbook into this node's workspaces and registers the bots.

Check: `axlr-bots node list` shows `sre-ro` `Online` with `role=sre-ro` and `dedicated`; `kubectl --kubeconfig ~/.config/axlr-sre/sre-ro/kubeconfig auth can-i get secrets -n shop` says `no`.

## Why the bus is direct

With NATS the broker maps each node to the user its certificate names, and this node has no certificate of its own over the socket. The control plane keeps the direct stream on beside NATS, so this node reads that one.

## What it cannot do

`kubectl diff` is a server-side dry run that needs `patch`, which this identity does not have; `kube-mcp` publishes `diff` only with writes. So triage diagnoses `drift` by comparing the live object with `desired/` by hand, the write node's `fix` bot proves it with `diff` and proposes the fix, and `sre-judge` leaves the scenario out of its default run. `top` is not a tool: `describe hpa` carries the current and target utilisation.
