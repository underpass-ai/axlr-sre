#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete networkpolicy deny-all --ignore-not-found
k -n "$ns" delete deploy payments --ignore-not-found --wait
echo "netpol: cleared"
