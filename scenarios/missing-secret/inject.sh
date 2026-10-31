#!/usr/bin/env bash
# orders reads DB_PASSWORD from the Secret db; the Secret is deleted and the
# pod is recreated: CreateContainerConfigError.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" create secret generic db --from-literal=password=lab-only-not-a-secret --dry-run=client -o yaml | k -n "$ns" apply -f -
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: orders, labels: {app: orders, lab/scenario: missing-secret}}
spec:
  replicas: 1
  selector: {matchLabels: {app: orders}}
  template:
    metadata: {labels: {app: orders}}
    spec:
      containers:
        - name: orders
          image: docker.io/library/busybox:1.37
          command: ["sh", "-c", "echo orders up; sleep 36000"]
          env:
            - name: DB_PASSWORD
              valueFrom: {secretKeyRef: {name: db, key: password}}
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {memory: 64Mi}}
YAML
k -n "$ns" rollout status deploy/orders --timeout 120s
k -n "$ns" delete secret db
k -n "$ns" delete pod -l app=orders --wait=false
echo "missing-secret: injected (Secret db deleted, orders pod recreated)"
