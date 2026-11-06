# The node sre-rw

The write node of the design (§7.2): a third `axlr-node` on the host, dedicated to the bots whose placement selects `role=sre-rw`, today `fix`. Its runs reach the cluster only through `kube-mcp` with writes on, as the ServiceAccount `axlr-sre-write`. In phase 1 `fix` proposes and applies nothing, so the node lets through one tool beyond the reads: `diff`, a server-side dry run.

```text
axlr-bots ──socket──▶ axlr-node --node-id sre-rw --dedicated --bus direct
                         │  --axlr-mcp-config ~/.config/axlr-sre/sre-rw/mcp.json
                         ▼
                      axlr-tui (fix, level propose) ──stdio──▶ kube-mcp ──▶ kubectl ──▶ API server
                                                       KUBE_WRITES=true        token of axlr-sre-write
                                                       KUBE_NAMESPACES=shop,drift-lab
```

Four gates decide each call, the first three before kubectl runs:

| Gate | What it lets through |
|---|---|
| the bot's `allowedTools` ([fix.json](../../bots/fix.json)) | the workspace reads, kube-mcp's reads and `diff` |
| the AXLR manifest `kube.json` (`allow_tools`) | the eight reads and `diff`: kube-mcp also publishes `apply`, `rollout_restart`, `rollout_undo` and `scale` with writes on, and the manifest keeps them from the model |
| kube-mcp's policy | the namespace allowlist, no Secrets, no blast-radius kinds in a manifest |
| RBAC ([rbac.yaml](rbac.yaml)) | the reads of `axlr-sre-read` across the cluster, and `patch` on Deployments, StatefulSets, HPAs and ConfigMaps only in the namespaces of `SRE_NAMESPACES` |

`patch` is there because `kubectl diff` is a dry-run patch. `update`, `scale` and `delete` of Pods (§7.2) come with the phase 2 tools that use them; decided on 6 November 2026.

| What | Where |
|---|---|
| ServiceAccount, read binding, write ClusterRole | [rbac.yaml](rbac.yaml), applied after [../sre-ro/rbac.yaml](../sre-ro/rbac.yaml) |
| write bindings | a RoleBinding `axlr-sre-write` in each namespace of `SRE_NAMESPACES`, made by `install.sh` with the namespace |
| kubeconfig with the token | `~/.config/axlr-sre/sre-rw/kubeconfig`, 0600 |
| kube-mcp's AXLR manifest | `~/.config/axlr-sre/sre-rw/kube.json`, plugin id `kube`, the reads and `diff` |
| the runs' `mcp.json` | `~/.config/axlr-sre/sre-rw/mcp.json`: kube only, `KUBE_WRITES=true`, and a `PATH` |
| state, traces, workspaces | `~/.local/state/axlr-node-sre-rw/` |
| unit | `~/.config/systemd/user/axlr-node-sre-rw.service` |

## Install

After `lab/up.sh` and `make -C tools install`:

```bash
nodes/sre-rw/install.sh
```

The steps are [../install-node.sh](../install-node.sh)'s, shared with sre-ro. It refuses to go on unless the identity can list pods and patch Deployments, and cannot read Secrets, update Deployments, delete Pods, create Namespaces or patch a Deployment in `kube-system`. Run it again to renew the token; a namespace dropped from `SRE_NAMESPACES` keeps its RoleBinding until it is deleted by hand. Then `bots/register.sh` registers `fix` and copies [fix-runbook.md](../../bots/fix-runbook.md) into its workspace.

Check: `axlr-bots node list` shows `sre-rw` `Online` with `role=sre-rw`; `kubectl --kubeconfig ~/.config/axlr-sre/sre-rw/kubeconfig auth can-i patch deployments -n kube-system` says `no`.

## PATH

AXLR starts an MCP server with exactly the environment its `mcp.json` entry gives, nothing inherited. `kubectl diff` runs the `diff` program, so the entry carries `PATH=/usr/local/bin:/usr/bin:/bin`; without it, on 6 November, every `diff` failed with `executable file not found in $PATH` and `fix` compared the objects by hand.
