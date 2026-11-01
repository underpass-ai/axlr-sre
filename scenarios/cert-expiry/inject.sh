#!/usr/bin/env bash
# A certificate that lives one hour (cert-manager's minimum), renewed only in
# its last ten minutes: it expires within the lab's two-hour threshold.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: cert-manager.io/v1
kind: Issuer
metadata: {name: lab-selfsigned, labels: {lab/scenario: cert-expiry}}
spec:
  selfSigned: {}
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata: {name: shop-tls, labels: {lab/scenario: cert-expiry}}
spec:
  secretName: shop-tls
  duration: 1h
  renewBefore: 10m
  dnsNames: [shop.lab.internal]
  issuerRef: {name: lab-selfsigned, kind: Issuer}
YAML
k -n "$ns" wait --for=condition=Ready certificate/shop-tls --timeout 120s
echo "cert-expiry: injected (Certificate shop-tls, duration 1h)"
