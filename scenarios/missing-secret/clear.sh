#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy orders --ignore-not-found --wait
k -n "$ns" delete secret db --ignore-not-found
echo "missing-secret: cleared"
