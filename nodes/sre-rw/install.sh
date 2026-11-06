#!/usr/bin/env bash
# Install the node sre-rw on this host (design §7.2): a third axlr-node,
# dedicated to the bots placed on role=sre-rw (fix), whose only cluster
# access is kube-mcp with writes on and the token of the ServiceAccount
# axlr-sre-write. Its AXLR manifest lets through the reads and diff only:
# phase 1 proposes, it applies nothing. Run it again to mint a fresh token.
# The environment it reads is in ../install-node.sh.
#
#   nodes/sre-rw/install.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
node=sre-rw
sa=axlr-sre-write
tools=(doctor discover get describe logs events rollout_status rollout_history diff)
writes=true
rbac_files=("$here/../sre-ro/rbac.yaml" "$here/rbac.yaml")
must_allow=("list pods" "patch deployments")
must_deny=("get secrets" "update deployments" "delete pods" "create namespaces" "patch deployments -n kube-system")
description="AXLR Bots node sre-rw (ServiceAccount axlr-sre-write: reads, and patch for diff)"

# The writes, namespace by namespace: each namespace of the allowlist is
# made if missing and gets its RoleBinding to the ClusterRole axlr-sre-write.
node_rbac() {
  local ns
  for ns in ${namespaces//,/ }; do
    k create namespace "$ns" --dry-run=client -o yaml | k apply -f - >/dev/null
    k create rolebinding axlr-sre-write -n "$ns" --clusterrole axlr-sre-write \
      --serviceaccount axlr-sre:axlr-sre-write --dry-run=client -o yaml | k apply -f - >/dev/null
  done
}
. "$here/../install-node.sh"
