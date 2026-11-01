#!/usr/bin/env bash
# Phase 0's judged run: every scenario, image-pull left firing an hour
# for the resends, Jev judging each diagnosis. The Jev key comes from the
# environment or from the node's env file; it is never printed.
#
#   judge/run.sh                       the whole run
#   judge/run.sh --only oom,drift      some scenarios, no soak
#   judge/run.sh --soak-for 0          the whole run without the hour
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${TYPESAFE_API_KEY:-}" ] && [ -r "$HOME/.config/axlr-bots/env" ]; then
  TYPESAFE_API_KEY="$(sed -n 's/^TYPESAFE_API_KEY=//p' "$HOME/.config/axlr-bots/env" | tail -1)"
  export TYPESAFE_API_KEY
fi
soak=(--soak image-pull --soak-for 60m)
for arg in "$@"; do
  case "$arg" in --only|--only=*|--soak|--soak=*|--soak-for|--soak-for=*) soak=() ;; esac
done
export GOFLAGS="${GOFLAGS:--p=2}"
exec go run ./cmd/sre-judge run "${soak[@]}" "$@"
