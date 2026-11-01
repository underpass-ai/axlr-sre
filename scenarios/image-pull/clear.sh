#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy catalog --ignore-not-found --wait
echo "image-pull: cleared"
