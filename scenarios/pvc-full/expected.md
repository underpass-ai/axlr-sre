# pvc-full: expected facts

Alert: `LabVolumeFull`, namespace `shop`, label `persistentvolumeclaim=uploads-data`.

| Fact | Value |
|---|---|
| object | PVC `uploads-data`, mounted at `/data` by Deployment `uploads` |
| namespace | `shop` |
| usage | 100 % (or above 95 %) of `64Mi` |
| cause | the `uploads` container wrote `uploads.bin` with `dd` until `No space left on device` |
| volume | PV `lab-uploads`, StorageClass `lab-local` (no provisioner: it cannot be expanded) |
| fix (not applied) | free space or move to a larger volume; expansion is not possible with this class |
