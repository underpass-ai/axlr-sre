#!/usr/bin/env bash
# Install the node sre-ro on this host (design §7.2): a second axlr-node,
# dedicated to the bots placed on role=sre-ro, whose only cluster access is
# kube-mcp read-only with the token of the ServiceAccount axlr-sre-read.
# Run it again to mint a fresh token; nothing it writes lands in the repo.
#
#   nodes/sre-ro/install.sh
#
# Environment:
#   ADMIN_CONTEXT    the admin context that applies rbac.yaml; default kind-sre-lab
#   SRE_NAMESPACES   kube-mcp's namespace allowlist; default shop
#   TOKEN_TTL        the token's life; default 720h (run again before it ends)
#   CONTROL          the control plane; default the host's socket
#   KUBE_MCP         the kube-mcp binary; default ~/.local/bin/kube-mcp
#   AXLR_NODE        the axlr-node binary; default ~/.local/bin/axlr-node
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
ctx="${ADMIN_CONTEXT:-kind-sre-lab}"
namespaces="${SRE_NAMESPACES:-shop}"
ttl="${TOKEN_TTL:-720h}"
control="${CONTROL:-unix://$HOME/.local/state/axlr-bots/axlr-bots.sock}"
kube_mcp="${KUBE_MCP:-$HOME/.local/bin/kube-mcp}"
axlr_node="${AXLR_NODE:-$HOME/.local/bin/axlr-node}"
kubectl_bin="$(command -v kubectl)"
conf="$HOME/.config/axlr-sre/sre-ro"
state="$HOME/.local/state/axlr-node-sre-ro"
unit="$HOME/.config/systemd/user/axlr-node-sre-ro.service"

for bin in "$kube_mcp" "$axlr_node" "$kubectl_bin"; do
  [ -x "$bin" ] || { echo "install.sh: $bin is not installed (kube-mcp: make -C tools install)" >&2; exit 1; }
done

k() { kubectl --context "$ctx" "$@"; }
k apply -f "$here/rbac.yaml" >/dev/null
token="$(k create token axlr-sre-read -n axlr-sre --duration "$ttl")"
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
  - name: axlr-sre-read
    user:
      token: $token
contexts:
  - name: sre-ro
    context:
      cluster: sre-lab
      user: axlr-sre-read
current-context: sre-ro
EOF

# The AXLR manifest of kube-mcp and the node's own mcp.json: the bots' runs
# load this file instead of the operator's, so kube is their only server.
jq -n --arg cmd "$kube_mcp" '{
    manifest_version: 1, id: "kube", command: $cmd, args: [],
    allow_tools: ["doctor", "discover", "get", "describe", "logs", "events", "rollout_status", "rollout_history"]
  }' >"$conf/kube.json"
jq -n --arg manifest "$conf/kube.json" --arg kubeconfig "$conf/kubeconfig" \
      --arg namespaces "$namespaces" --arg kubectl "$kubectl_bin" '{
    version: 1,
    plugins: [{
      name: "kube", purpose: "tools", approval: "manual", manifest: $manifest,
      env: {KUBECONFIG: $kubeconfig, KUBE_CONTEXT: "sre-ro", KUBE_NAMESPACES: $namespaces, KUBECTL: $kubectl}
    }]
  }' >"$conf/mcp.json"

# kube-mcp's doctor with this identity, before any bot runs on it.
KUBECONFIG="$conf/kubeconfig" kubectl auth can-i get secrets -n "${namespaces%%,*}" >/dev/null 2>&1 \
  && { echo "install.sh: axlr-sre-read can read Secrets; check rbac.yaml" >&2; exit 1; }
KUBECONFIG="$conf/kubeconfig" kubectl auth can-i list pods -n "${namespaces%%,*}" >/dev/null \
  || { echo "install.sh: axlr-sre-read cannot list pods" >&2; exit 1; }

# The bus is direct: the NATS user of a node is its certificate's, and this
# node has none of its own.
mkdir -p "$(dirname "$unit")"
umask 022
cat >"$unit" <<EOF
[Unit]
Description=AXLR Bots node sre-ro (read-only ServiceAccount axlr-sre-read)
After=axlr-bots.service
Wants=axlr-bots.service

[Service]
Type=simple
EnvironmentFile=-$HOME/.config/axlr-bots/env
Environment=KUBECONFIG=$conf/kubeconfig
ExecStart=$axlr_node --control $control --node-id sre-ro --labels role=sre-ro,host=%l --dedicated --bus direct --state $state --axlr-mcp-config $conf/mcp.json
Restart=on-failure
RestartSec=5s
TimeoutStopSec=30s
KillMode=mixed
NoNewPrivileges=yes

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable axlr-node-sre-ro.service >/dev/null 2>&1
systemctl --user restart axlr-node-sre-ro.service
echo "install.sh: node sre-ro on $control, kube-mcp in $namespaces as axlr-sre-read (token for $ttl)"
