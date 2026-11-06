#!/usr/bin/env bash
# The 29 Oct drift bot's lab on sre-lab: the desired state (desired/) goes to
# the workspaces of the bots that read it, triage and diag-k8s on sre-ro and
# fix on sre-rw, is applied, then drifts in the two fields it declares.
. "$(dirname "$0")/../lib.sh"
desired="$(cd "$(dirname "$0")" && pwd)/desired"
state="${AXLR_NODE_STATE:-$HOME/.local/state}"
for place in axlr-node-sre-ro/workspaces/triage axlr-node-sre-ro/workspaces/diag-k8s axlr-node-sre-rw/workspaces/fix; do
  mkdir -p "$state/$place/desired"
  cp "$desired/deployment.yaml" "$state/$place/desired/deployment.yaml"
done
k create namespace drift-lab --dry-run=client -o yaml | k apply -f - >/dev/null
k apply -f "$desired/deployment.yaml"
k -n drift-lab rollout status deploy/web --timeout 180s
k -n drift-lab scale deploy/web --replicas=3
k -n drift-lab set image deploy/web web=docker.io/library/nginx:1.26-alpine
echo "drift: injected (drift-lab/web: 3 replicas, nginx 1.26-alpine; desired 2, 1.27-alpine)"
