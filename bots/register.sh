#!/usr/bin/env bash
# Register (or update) triage and diag-k8s on the control plane and enable
# them. The trigger's secret comes from lab/.secret, made by lab/up.sh, and
# never lands in the repo.
#
# bots/runbook.md goes into each bot's workspace on this node, where the bot
# reads it (local_read) before its first kubectl call.
#
# Environment:
#   NODE        the node the bots run on; default: this host's node (gx10-ff95 in the run)
#   AXLR_BOTS   the CLI; default axlr-bots on the PATH
#   AXLR_NODE_WORKSPACES   the node's workspaces; default ~/.local/state/axlr-node/workspaces
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cli="${AXLR_BOTS:-axlr-bots}"
node="${NODE:-}"
workspaces="${AXLR_NODE_WORKSPACES:-$HOME/.local/state/axlr-node/workspaces}"
secret_file="$here/../lab/.secret"
[ -s "$secret_file" ] || { echo "register.sh: run lab/up.sh first (no lab/.secret)" >&2; exit 1; }
secret="$(cat "$secret_file")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for bot in diag-k8s triage; do
  mkdir -p "$workspaces/$bot"
  cp "$here/runbook.md" "$workspaces/$bot/runbook.md"
  jq --arg secret "$secret" --arg node "$node" '
      (.triggers[] | select(.type == "alertmanager") | .secret) |= $secret
      | if $node != "" then .placement.nodes = [$node] else . end' \
    "$here/$bot.json" >"$work/$bot.json"
  if "$cli" bot get "$bot" >/dev/null 2>&1; then
    jq 'del(.id, .tenant, .name, .owner)' "$work/$bot.json" >"$work/$bot.patch.json"
    "$cli" bot update "$bot" --file "$work/$bot.patch.json" >/dev/null
    echo "register.sh: updated $bot"
  else
    "$cli" bot register --file "$work/$bot.json" >/dev/null
    echo "register.sh: registered $bot"
  fi
  "$cli" bot enable "$bot" >/dev/null 2>&1 || true
  "$cli" bot get "$bot" | jq -c '{id, status, level, groups, triggers: [.triggers[] | {type, name}]}'
done
