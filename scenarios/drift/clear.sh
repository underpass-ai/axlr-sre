#!/usr/bin/env bash
# The namespace stays: it holds sre-rw's RoleBinding (nodes/sre-rw/install.sh).
. "$(dirname "$0")/../lib.sh"
k -n drift-lab delete deployment web --ignore-not-found --wait
echo "drift: cleared"
