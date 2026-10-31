# Sourced by every scenario: the lab's context and helpers.
set -euo pipefail
ctx=kind-sre-lab
k() { kubectl --context "$ctx" "$@"; }
ns=shop
# the node the node and volume faults use; the lab has one node
fault_node="${LAB_FAULT_NODE:-sre-lab-control-plane}"
k create namespace "$ns" --dry-run=client -o yaml | k apply -f - >/dev/null
