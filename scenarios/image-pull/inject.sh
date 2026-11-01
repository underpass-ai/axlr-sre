#!/usr/bin/env bash
# A tag that does not exist in the registry.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: catalog, labels: {app: catalog, lab/scenario: image-pull}}
spec:
  replicas: 1
  selector: {matchLabels: {app: catalog}}
  template:
    metadata: {labels: {app: catalog}}
    spec:
      containers:
        - name: catalog
          image: docker.io/library/nginx:1.99
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {memory: 64Mi}}
YAML
echo "image-pull: injected (deploy/catalog on nginx:1.99)"
