#!/usr/bin/env bash
# Stop the kubelet of the lab's node: it goes NotReady. Its containers keep
# running (the kubelet does not stop them), so Prometheus and Alertmanager
# still see and send the alert.
. "$(dirname "$0")/../lib.sh"
docker exec "$fault_node" systemctl stop kubelet
echo "node-notready: injected (kubelet stopped on $fault_node)"
