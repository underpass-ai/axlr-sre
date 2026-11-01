#!/usr/bin/env bash
# A worker that holds 60 MiB under a 32 MiB memory limit: OOMKilled, restarted.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: cart, labels: {app: cart, lab/scenario: oom}}
spec:
  replicas: 1
  selector: {matchLabels: {app: cart}}
  template:
    metadata: {labels: {app: cart}}
    spec:
      containers:
        - name: cart
          image: docker.io/library/python:3.13-alpine
          command: ["python", "-c", "import time; print('cart: loading 60 MiB cache', flush=True); cache = bytearray(60 * 1024 * 1024); cache[::4096] = b'x' * len(cache[::4096]); print('cart: cache loaded', flush=True); time.sleep(36000)"]
          resources: {requests: {cpu: 10m, memory: 32Mi}, limits: {memory: 32Mi}}
YAML
echo "oom: injected (deploy/cart, limit 32Mi, needs 60Mi)"
