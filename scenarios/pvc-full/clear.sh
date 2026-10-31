#!/usr/bin/env bash
. "$(dirname "$0")/../lib.sh"
k -n "$ns" delete deploy uploads --ignore-not-found --wait
k -n "$ns" delete pvc uploads-data --ignore-not-found --wait
k delete pv lab-uploads --ignore-not-found --wait
k delete storageclass lab-local --ignore-not-found
docker exec "$fault_node" sh -c 'mountpoint -q /mnt/lab-uploads && umount /mnt/lab-uploads || true'
echo "pvc-full: cleared"
