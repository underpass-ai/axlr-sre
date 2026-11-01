#!/usr/bin/env bash
# Delete the lab cluster. The trigger's secret (lab/.secret) stays, so the
# registered triage bot keeps matching a lab brought up again.
set -euo pipefail
cluster=sre-lab
if kind get clusters | grep -qx "$cluster"; then
  kind delete cluster --name "$cluster"
else
  echo "down.sh: no cluster $cluster"
fi
