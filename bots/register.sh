#!/usr/bin/env bash
# Register (or update) the sre team's bots on the control plane and enable
# them. The trigger's secret comes from lab/.secret, made by lab/up.sh, and
# never lands in the repo.
#
# Each bot reads its runbook (local_read runbook.md) before its first
# cluster call; this script copies it into the bot's workspace on its node:
#
#   triage, diag-k8s   node sre-ro (nodes/sre-ro/install.sh)   bots/runbook.md
#   fix                node sre-rw (nodes/sre-rw/install.sh)   bots/fix-runbook.md
#
# Environment:
#   AXLR_BOTS          the CLI; default axlr-bots on the PATH
#   AXLR_NODE_STATE    where the nodes keep their state; default ~/.local/state
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cli="${AXLR_BOTS:-axlr-bots}"
state="${AXLR_NODE_STATE:-$HOME/.local/state}"
secret_file="$here/../lab/.secret"
[ -s "$secret_file" ] || { echo "register.sh: run lab/up.sh first (no lab/.secret)" >&2; exit 1; }
secret="$(cat "$secret_file")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# bot node runbook; a delegate goes before the bot that delegates to it.
while read -r bot node runbook; do
  workspace="$state/axlr-node-$node/workspaces/$bot"
  mkdir -p "$workspace"
  cp "$here/$runbook" "$workspace/runbook.md"
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
done <<'EOF'
diag-k8s sre-ro runbook.md
triage sre-ro runbook.md
fix sre-rw fix-runbook.md
EOF
