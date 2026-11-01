#!/usr/bin/env bash
# payments checks DNS in its readiness probe; a NetworkPolicy deny-all on
# the namespace cuts every egress, DNS (53/UDP) included: running, not ready.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: payments, labels: {app: payments, lab/scenario: netpol}}
spec:
  replicas: 1
  selector: {matchLabels: {app: payments}}
  template:
    metadata: {labels: {app: payments}}
    spec:
      containers:
        - name: payments
          image: docker.io/library/busybox:1.37
          command: ["sh", "-c", "echo payments up; sleep 36000"]
          readinessProbe:
            exec: {command: ["nslookup", "kubernetes.default.svc.cluster.local"]}
            periodSeconds: 10
            timeoutSeconds: 5
            failureThreshold: 2
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {memory: 64Mi}}
YAML
k -n "$ns" rollout status deploy/payments --timeout 120s
k -n "$ns" apply -f - <<'YAML'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: {name: deny-all, labels: {lab/scenario: netpol}}
spec:
  podSelector: {}
  policyTypes: [Egress]
YAML
echo "netpol: injected (NetworkPolicy deny-all in shop)"
