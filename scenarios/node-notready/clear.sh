#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
docker exec "$fault_node" systemctl start kubelet
k wait --for=condition=Ready "node/$fault_node" --timeout 180s
echo "node-notready: cleared"
