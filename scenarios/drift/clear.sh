#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k delete namespace drift-lab --ignore-not-found --wait
echo "drift: cleared"
