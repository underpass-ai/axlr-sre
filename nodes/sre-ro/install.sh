#!/usr/bin/env bash
# Install the node sre-ro on this host (design §7.2): a second axlr-node,
# dedicated to the bots placed on role=sre-ro, whose only cluster access is
# kube-mcp read-only with the token of the ServiceAccount axlr-sre-read.
# Run it again to mint a fresh token. The environment it reads is in
# ../install-node.sh.
#
#   nodes/sre-ro/install.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
node=sre-ro
sa=axlr-sre-read
tools=(doctor discover get describe logs events rollout_status rollout_history)
writes=false
must_allow=("list pods")
must_deny=("get secrets")
description="AXLR Bots node sre-ro (read-only ServiceAccount axlr-sre-read)"
. "$here/../install-node.sh"
