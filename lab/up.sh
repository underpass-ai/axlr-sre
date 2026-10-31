#!/usr/bin/env bash
# Bring up the lab: a kind cluster sre-lab, metrics-server, cert-manager and
# kube-prometheus-stack, whose Alertmanager posts the lab's alerts to the
# control plane's alertmanager trigger (bot triage, trigger alerts).
#
# Needs: docker, kind, helm, kubectl, openssl; the control plane serving its
# operator API on an address the kind network reaches (--api-listen
# 0.0.0.0:7879) and its fleet CA.
#
# Environment:
#   CP_HOST   the address Alertmanager dials; default: the kind network's gateway
#   CP_PORT   the operator API's port; default 7879
#   AXLR_CA   the fleet CA; default ~/.local/state/axlr-bots/ca/ca.crt
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cluster=sre-lab
ctx="kind-$cluster"
k() { kubectl --context "$ctx" "$@"; }

for tool in docker kind helm kubectl openssl; do
  command -v "$tool" >/dev/null || { echo "up.sh: $tool is missing" >&2; exit 1; }
done

ca="${AXLR_CA:-$HOME/.local/state/axlr-bots/ca/ca.crt}"
[ -r "$ca" ] || { echo "up.sh: fleet CA not found at $ca" >&2; exit 1; }

# The trigger's secret: made once, kept beside the lab, never committed.
secret_file="$here/.secret"
if [ ! -s "$secret_file" ]; then
  umask 077
  openssl rand -hex 24 >"$secret_file"
fi
secret="$(cat "$secret_file")"

if ! kind get clusters | grep -qx "$cluster"; then
  kind create cluster --config "$here/kind.yaml"
else
  echo "up.sh: cluster $cluster already exists"
fi

host="${CP_HOST:-$(docker network inspect kind -f '{{range .IPAM.Config}}{{if .Gateway}}{{.Gateway}} {{end}}{{end}}' | tr ' ' '\n' | grep -E '^[0-9]+\.' | head -1)}"
port="${CP_PORT:-7879}"
url="https://$host:$port/v1/triggers/triage/alerts"
echo "up.sh: Alertmanager will post to $url"

helm repo add prometheus-community https://prometheus-community.github.io/helm-charts >/dev/null 2>&1 || true
helm repo add jetstack https://charts.jetstack.io >/dev/null 2>&1 || true
helm repo add metrics-server https://kubernetes-sigs.github.io/metrics-server/ >/dev/null 2>&1 || true
helm repo update >/dev/null

# metrics-server: the HPA of hpa-maxed needs CPU metrics; kind's kubelets
# serve self-signed certificates.
helm upgrade --install metrics-server metrics-server/metrics-server \
  --kube-context "$ctx" -n kube-system --version 3.14.0 \
  --set 'args={--kubelet-insecure-tls}' --wait

k create namespace monitoring --dry-run=client -o yaml | k apply -f -
k -n monitoring create secret generic axlr-ca --from-file=ca.crt="$ca" --dry-run=client -o yaml | k apply -f -

# kube-prometheus-stack first: cert-manager's ServiceMonitor needs its CRDs.
values="$(mktemp)"
trap 'rm -f "$values"' EXIT
sed -e "s|@CP_URL@|$url|" -e "s|@SECRET@|$secret|" "$here/kube-prometheus-stack.yaml" >"$values"
helm upgrade --install kps prometheus-community/kube-prometheus-stack \
  --kube-context "$ctx" -n monitoring --version 92.17.0 -f "$values" --wait --timeout 10m

# cert-manager: cert-expiry issues a one-hour certificate; its metrics feed
# LabCertificateExpiringSoon.
helm upgrade --install cert-manager jetstack/cert-manager \
  --kube-context "$ctx" -n cert-manager --create-namespace --version v1.21.3 \
  --set crds.enabled=true \
  --set prometheus.enabled=true \
  --set prometheus.servicemonitor.enabled=true \
  --set prometheus.servicemonitor.honorLabels=true \
  --wait

k apply -f "$here/rules.yaml"
k create namespace shop --dry-run=client -o yaml | k apply -f -

k -n monitoring rollout status statefulset/alertmanager-kps-alertmanager --timeout 5m
k -n monitoring rollout status statefulset/prometheus-kps-prometheus --timeout 5m

# The receiver can reach the control plane: without the secret the trigger
# answers 403, which proves the address, the TLS and the route.
code="$(k -n monitoring run lab-reach --rm -i --restart=Never --quiet \
  --image=docker.io/curlimages/curl:8.11.1 \
  -- curl -s -o /dev/null -w '%{http_code}' -k -X POST -d '{}' "$url" 2>/dev/null || true)"
echo "up.sh: control plane answered $code without the secret (403 expected)"

echo "up.sh: ready. kubectl --context $ctx; the secret for bots/register.sh is in lab/.secret"
