#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy cart --ignore-not-found --wait
echo "oom: cleared"
