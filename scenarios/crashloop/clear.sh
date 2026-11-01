#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy checkout --ignore-not-found --wait
echo "crashloop: cleared"
