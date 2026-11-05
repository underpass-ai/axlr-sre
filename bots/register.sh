#!/usr/bin/env bash
# Register (or update) triage and diag-k8s on the control plane and enable
# them. The trigger's secret comes from lab/.secret, made by lab/up.sh, and
# never lands in the repo.
#
# bots/runbook.md goes into each bot's workspace on this node, where the bot
# reads it (local_read) before its first cluster call.
#
# The bots run on the node sre-ro (placement selector role=sre-ro), which
# nodes/sre-ro/install.sh sets up; its workspaces hold the runbook.
#
# Environment:
#   AXLR_BOTS   the CLI; default axlr-bots on the PATH
#   AXLR_NODE_WORKSPACES   the node's workspaces; default ~/.local/state/axlr-node-sre-ro/workspaces
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cli="${AXLR_BOTS:-axlr-bots}"
workspaces="${AXLR_NODE_WORKSPACES:-$HOME/.local/state/axlr-node-sre-ro/workspaces}"
secret_file="$here/../lab/.secret"
[ -s "$secret_file" ] || { echo "register.sh: run lab/up.sh first (no lab/.secret)" >&2; exit 1; }
secret="$(cat "$secret_file")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for bot in diag-k8s triage; do
  mkdir -p "$workspaces/$bot"
  cp "$here/runbook.md" "$workspaces/$bot/runbook.md"
  jq --arg secret "$secret" '(.triggers[] | select(.type == "alertmanager") | .secret) |= $secret' \
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
