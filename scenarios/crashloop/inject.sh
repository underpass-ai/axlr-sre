#!/usr/bin/env bash
# A release that exits with code 1: checkout runs fine on nginx (revision 1),
# then a new revision ships a command that cannot find its config file.
. "$(dirname "$0")/../lib.sh"
k -n "$ns" apply -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: {name: checkout, labels: {app: checkout, lab/scenario: crashloop}}
spec:
  replicas: 1
  selector: {matchLabels: {app: checkout}}
  template:
    metadata: {labels: {app: checkout}}
    spec:
      containers:
        - name: checkout
          image: docker.io/library/busybox:1.37
          command: ["sh", "-c", "echo checkout v1 serving; sleep 36000"]
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {memory: 64Mi}}
YAML
k -n "$ns" rollout status deploy/checkout --timeout 120s
k -n "$ns" annotate deploy/checkout kubernetes.io/change-cause="release v2" --overwrite
k -n "$ns" patch deploy/checkout --type json -p '[{"op":"replace","path":"/spec/template/spec/containers/0/command","value":["sh","-c","echo checkout v2 starting; echo FATAL: config file /etc/checkout/config.yaml not found >&2; exit 1"]}]'
echo "crashloop: injected (deploy/checkout revision 2 exits 1)"
