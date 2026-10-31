#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy load search --ignore-not-found --wait
k -n "$ns" delete hpa search --ignore-not-found
k -n "$ns" delete service search --ignore-not-found
echo "hpa-maxed: cleared"
