#!/usr/bin/env bash
# The 29 Oct drift bot's lab (axlr-bots examples/k8s-drift) on sre-lab: the
# desired state goes to the bots' workspaces, is applied, then drifts in the
# two fields the manifest declares.
. "$(dirname "$0")/../lib.sh"
example="${AXLR_BOTS_DIR:-$HOME/Documents/ai/axlr-bots}/examples/k8s-drift/desired/deployment.yaml"
workspaces="${AXLR_NODE_WORKSPACES:-$HOME/.local/state/axlr-node/workspaces}"
[ -r "$example" ] || { echo "drift: $example not found (set AXLR_BOTS_DIR)" >&2; exit 1; }
for bot in triage diag-k8s; do
  mkdir -p "$workspaces/$bot/desired"
  cp "$example" "$workspaces/$bot/desired/deployment.yaml"
done
k apply -f "$example"
k -n drift-lab rollout status deploy/web --timeout 180s
k -n drift-lab scale deploy/web --replicas=3
k -n drift-lab set image deploy/web web=docker.io/library/nginx:1.26-alpine
echo "drift: injected (drift-lab/web: 3 replicas, nginx 1.26-alpine; desired 2, 1.27-alpine)"
