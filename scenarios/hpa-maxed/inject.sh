#!/usr/bin/env bash
# search under load: three wget loops against it; its HPA (max 3, CPU 50 %)
# scales to its maximum and stays there.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: search, labels: {app: search, lab/scenario: hpa-maxed}}
spec:
  replicas: 1
  selector: {matchLabels: {app: search}}
  template:
    metadata: {labels: {app: search}}
    spec:
      containers:
        - name: search
          image: docker.io/library/nginx:1.27-alpine
          ports: [{containerPort: 80}]
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {cpu: 200m, memory: 64Mi}}
---
apiVersion: v1
kind: Service
metadata: {name: search, labels: {lab/scenario: hpa-maxed}}
spec:
  selector: {app: search}
  ports: [{port: 80, targetPort: 80}]
---
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata: {name: search, labels: {lab/scenario: hpa-maxed}}
spec:
  scaleTargetRef: {apiVersion: apps/v1, kind: Deployment, name: search}
  minReplicas: 1
  maxReplicas: 3
  metrics:
    - type: Resource
      resource: {name: cpu, target: {type: Utilization, averageUtilization: 50}}
---
apiVersion: apps/v1
kind: Deployment
metadata: {name: load, labels: {app: load, lab/scenario: hpa-maxed}}
spec:
  replicas: 1
  selector: {matchLabels: {app: load}}
  template:
    metadata: {labels: {app: load}}
    spec:
      containers:
        - name: load
          image: docker.io/library/busybox:1.37
          command: ["sh", "-c", "for i in 1 2 3; do (while true; do wget -q -O /dev/null http://search; done) & done; wait"]
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {cpu: 500m, memory: 64Mi}}
YAML
echo "hpa-maxed: injected (deploy/search + HPA max 3 + deploy/load)"
