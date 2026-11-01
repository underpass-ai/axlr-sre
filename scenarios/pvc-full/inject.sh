#!/usr/bin/env bash
# uploads writes with dd until its 64 MiB volume is full. The volume is a
# local PersistentVolume on a 64 MiB tmpfs inside the kind node, so the
# filesystem really fills (kind's local-path volumes would fill the host disk
# and report no volume metrics). tmpfs pages count against the memory of the
# container that writes them: its limit must hold the 64 MiB, or the kernel
# kills the container before the volume is full.
. "$(dirname "$0")/../lib.sh"
node="$fault_node"
docker exec "$node" sh -c 'mkdir -p /mnt/lab-uploads && (mountpoint -q /mnt/lab-uploads || mount -t tmpfs -o size=64m tmpfs /mnt/lab-uploads)'
k apply -f - <<YAML
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata: {name: lab-local, labels: {lab/scenario: pvc-full}}
provisioner: kubernetes.io/no-provisioner
volumeBindingMode: WaitForFirstConsumer
---
apiVersion: v1
kind: PersistentVolume
metadata: {name: lab-uploads, labels: {lab/scenario: pvc-full}}
spec:
  capacity: {storage: 64Mi}
  accessModes: [ReadWriteOnce]
  persistentVolumeReclaimPolicy: Retain
  storageClassName: lab-local
  local: {path: /mnt/lab-uploads}
  nodeAffinity:
    required:
      nodeSelectorTerms:
        - matchExpressions:
            - {key: kubernetes.io/hostname, operator: In, values: [$node]}
YAML
k -n "$ns" apply -f - <<'YAML'
apiVersion: v1
kind: PersistentVolumeClaim
metadata: {name: uploads-data, labels: {lab/scenario: pvc-full}}
spec:
  accessModes: [ReadWriteOnce]
  storageClassName: lab-local
  resources: {requests: {storage: 64Mi}}
---
apiVersion: apps/v1
kind: Deployment
metadata: {name: uploads, labels: {app: uploads, lab/scenario: pvc-full}}
spec:
  replicas: 1
  selector: {matchLabels: {app: uploads}}
  template:
    metadata: {labels: {app: uploads}}
    spec:
      containers:
        - name: uploads
          image: docker.io/library/busybox:1.37
          command: ["sh", "-c", "echo uploads: writing; dd if=/dev/zero of=/data/uploads.bin bs=1M count=200; echo uploads: write failed; df -h /data; sleep 36000"]
          volumeMounts: [{name: data, mountPath: /data}]
          resources: {requests: {cpu: 10m, memory: 16Mi}, limits: {memory: 256Mi}}
      volumes:
        - name: data
          persistentVolumeClaim: {claimName: uploads-data}
YAML
echo "pvc-full: injected (deploy/uploads filling PVC uploads-data, 64Mi)"
