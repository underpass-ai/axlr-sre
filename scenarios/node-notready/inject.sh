#!/usr/bin/env bash
# Stop the kubelet of the worker sre-lab-worker2: the node goes NotReady.
# The monitoring stack runs on the other worker.
. "$(dirname "$0")/../lib.sh"
docker exec "$fault_node" systemctl stop kubelet
echo "node-notready: injected (kubelet stopped on $fault_node)"
