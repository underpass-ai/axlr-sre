#!/usr/bin/env bash
# Sourced by nodes/<node>/install.sh: install one node of the design (§7.2)
# on this host. A node is a second axlr-node, dedicated to the bots placed
# on role=<node>, whose only cluster access is kube-mcp with the token of
# the node's own ServiceAccount. Nothing it writes lands in the repo.
#
# The node's install.sh sets, before sourcing this file:
#   node         the node id and its role label: sre-ro, sre-rw
#   sa           the ServiceAccount in namespace axlr-sre, from the node's rbac.yaml
#   tools        kube-mcp's tools the node's AXLR manifest lets through
#   writes       true publishes kube-mcp's write tools (KUBE_WRITES)
#   must_allow   kubectl auth can-i checks the identity must pass ("list pods")
#   must_deny    checks it must fail ("get secrets", "patch deployments -n kube-system")
#   description  the unit's one line
#   rbac_files   optional: the manifests to apply; default the node's rbac.yaml
#   node_rbac    optional function, run after them with k() and $namespaces
#
# Environment:
#   ADMIN_CONTEXT    the admin context that applies rbac.yaml; default kind-sre-lab
#   SRE_NAMESPACES   kube-mcp's namespace allowlist; default shop,drift-lab
#   TOKEN_TTL        the token's life; default 720h (run again before it ends)
#   CONTROL          the control plane; default the host's socket
#   KUBE_MCP         the kube-mcp binary; default ~/.local/bin/kube-mcp
#   AXLR_NODE        the axlr-node binary; default ~/.local/bin/axlr-node
set -euo pipefail

ctx="${ADMIN_CONTEXT:-kind-sre-lab}"
namespaces="${SRE_NAMESPACES:-shop,drift-lab}"
ttl="${TOKEN_TTL:-720h}"
control="${CONTROL:-unix://$HOME/.local/state/axlr-bots/axlr-bots.sock}"
kube_mcp="${KUBE_MCP:-$HOME/.local/bin/kube-mcp}"
axlr_node="${AXLR_NODE:-$HOME/.local/bin/axlr-node}"
kubectl_bin="$(command -v kubectl)"
conf="$HOME/.config/axlr-sre/$node"
state="$HOME/.local/state/axlr-node-$node"
unit="$HOME/.config/systemd/user/axlr-node-$node.service"

for bin in "$kube_mcp" "$axlr_node" "$kubectl_bin"; do
  [ -x "$bin" ] || { echo "install.sh: $bin is not installed (kube-mcp: make -C tools install)" >&2; exit 1; }
done
if [ "$writes" = true ] && ! PATH=/usr/local/bin:/usr/bin:/bin command -v diff >/dev/null; then
  echo "install.sh: kubectl diff needs the diff program in /usr/local/bin, /usr/bin or /bin" >&2; exit 1
fi

k() { kubectl --context "$ctx" "$@"; }
declare -p rbac_files >/dev/null 2>&1 || rbac_files=("$here/rbac.yaml")
for file in "${rbac_files[@]}"; do k apply -f "$file" >/dev/null; done
if declare -F node_rbac >/dev/null; then node_rbac; fi
token="$(k create token "$sa" -n axlr-sre --duration "$ttl")"
server="$(kubectl config view --raw --minify --context "$ctx" -o jsonpath='{.clusters[0].cluster.server}')"
ca="$(kubectl config view --raw --minify --context "$ctx" -o jsonpath='{.clusters[0].cluster.certificate-authority-data}')"

umask 077
mkdir -p "$conf" "$state"
cat >"$conf/kubeconfig" <<EOF
apiVersion: v1
kind: Config
clusters:
  - name: sre-lab
    cluster:
      server: $server
      certificate-authority-data: $ca
users:
  - name: $sa
    user:
      token: $token
contexts:
  - name: $node
    context:
      cluster: sre-lab
      user: $sa
current-context: $node
EOF

# The AXLR manifest of kube-mcp and the node's own mcp.json: the bots' runs
# load this file instead of the operator's, so kube is their only server,
# and only the tools the manifest names reach the model. AXLR starts the
# server with exactly this environment, nothing inherited: kubectl diff runs
# the diff program, so PATH is given (on 6 November its absence failed
# every diff with "executable file not found in $PATH"). Without HOME kubectl
# keeps its discovery cache under the working directory, the bot's
# workspace, so KUBECACHEDIR puts it in the node's state.
jq -n --arg cmd "$kube_mcp" --args '{
    manifest_version: 1, id: "kube", command: $cmd, args: [], allow_tools: $ARGS.positional
  }' "${tools[@]}" >"$conf/kube.json"
jq -n --arg manifest "$conf/kube.json" --arg kubeconfig "$conf/kubeconfig" --arg node "$node" \
      --arg namespaces "$namespaces" --arg kubectl "$kubectl_bin" --arg writes "$writes" --arg cache "$state/kube-cache" '{
    version: 1,
    plugins: [{
      name: "kube", purpose: "tools", approval: "manual", manifest: $manifest,
      env: {KUBECONFIG: $kubeconfig, KUBE_CONTEXT: $node, KUBE_NAMESPACES: $namespaces, KUBECTL: $kubectl,
            KUBE_WRITES: $writes, PATH: "/usr/local/bin:/usr/bin:/bin", KUBECACHEDIR: $cache}
    }]
  }' >"$conf/mcp.json"

# The identity, before any bot runs on it: what it must do and what it
# must not, in the first namespace of the allowlist unless the check names
# its own (a later -n wins).
first="${namespaces%%,*}"
for check in "${must_allow[@]}"; do
  # shellcheck disable=SC2086 # the check is several words on purpose
  KUBECONFIG="$conf/kubeconfig" kubectl auth can-i -n "$first" $check >/dev/null \
    || { echo "install.sh: $sa cannot $check; check rbac.yaml" >&2; exit 1; }
done
for check in "${must_deny[@]}"; do
  # shellcheck disable=SC2086
  KUBECONFIG="$conf/kubeconfig" kubectl auth can-i -n "$first" $check >/dev/null 2>&1 \
    && { echo "install.sh: $sa can $check; check rbac.yaml" >&2; exit 1; }
done

# The bus is direct: the NATS user of a node is its certificate's, and this
# node has none of its own.
mkdir -p "$(dirname "$unit")"
umask 022
cat >"$unit" <<EOF
[Unit]
Description=$description
After=axlr-bots.service
Wants=axlr-bots.service

[Service]
Type=simple
EnvironmentFile=-$HOME/.config/axlr-bots/env
Environment=KUBECONFIG=$conf/kubeconfig
ExecStart=$axlr_node --control $control --node-id $node --labels role=$node,host=%l --dedicated --bus direct --state $state --axlr-mcp-config $conf/mcp.json
Restart=on-failure
RestartSec=5s
TimeoutStopSec=30s
KillMode=mixed
NoNewPrivileges=yes

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable "axlr-node-$node.service" >/dev/null 2>&1
systemctl --user restart "axlr-node-$node.service"
echo "install.sh: node $node on $control, kube-mcp in $namespaces as $sa (token for $ttl, writes $writes)"
