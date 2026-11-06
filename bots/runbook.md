# Runbook of the sre team (phase 1, read-only)

Read before the first cluster call. The cluster is reached only through the `kube` server's tools,
read-only, as the ServiceAccount `axlr-sre-read`: `discover`, `get`, `describe`, `logs`, `events`,
`rollout_history`. Call them with `axlr_call_tool` by that exact name: `describe`, never
`kube/describe` (AXLR rejects the prefixed name as an unknown tool). Every call names its `namespace` (the alert's); for cluster-scoped kinds
(`node`, `pv`, `storageclass`) pass the alert's namespace too: it is ignored. Secrets are never
read: a missing one is named in the pod's events and in `describe`. Change nothing.

## Any alert

`get` and `describe` the object the alert names; `events` of the namespace (Warning first, then
all with `warnings_only` false when Warnings say little); `logs` of the failing pod (`previous`
true after a restart); then what it depends on: ReplicaSets, PVC and PV, NetworkPolicies, HPA,
Certificate and Issuer, the Node's conditions and taints. `get` gives summaries; `raw` true
gives the whole objects when a field is needed (a command, an image, a limit, a policy's rules).

## CrashLoopBackOff: what changed

    rollout_history {kind: deployment, name: <name>, namespace: <ns>}        -> revisions and CHANGE-CAUSE
    get {kind: replicaset, selector: app=<app>, namespace: <ns>, raw: true}   -> each revision's template

Each ReplicaSet carries its revision in `metadata.annotations["deployment.kubernetes.io/revision"]`.
Report: which revision introduced the change (its number and change-cause, e.g. revision 2 "release v2"),
what it changed (command or image, old and new), and that the previous revision ran (its template is the
one that ran before; its replicas were scaled to 0 by the rollout).
FIX (a write, never run here): roll back to the last revision that ran (`kubectl rollout undo -n <ns> deploy/<name> --to-revision=<n>`)
or ship what the new revision needs.

## Volume full

    describe {kind: pvc, name: <claim>, namespace: <ns>}            -> Capacity, Status, Volume, Used By
    describe {kind: pv, name: <volume>, namespace: <ns>}            -> StorageClass, Capacity, Source
    describe {kind: storageclass, name: <class>, namespace: <ns>}   -> Provisioner, AllowVolumeExpansion

Report: usage (from the pod's logs), who writes (the container's command and log lines), the PV's name,
the StorageClass's name and provisioner, and whether the volume can be expanded:
provisioner `kubernetes.io/no-provisioner` with AllowVolumeExpansion unset cannot.
FIX: free space or move to a larger volume; expansion is not possible with such a class.

## HPA at its maximum: where the load comes from

    describe {kind: hpa, name: <hpa>, namespace: <ns>}                 -> Min/Max replicas, utilisation / target, ScalingLimited
    logs {pod: <a pod of the scaled workload>, namespace: <ns>, tail: 50}   -> the callers: client IP and user agent
    get {kind: pod, namespace: <ns>}                                    -> each pod's ip: the client IP's pod
    describe {kind: deployment, name: <the client's deployment>, namespace: <ns>}   -> its command: what it calls

Report: current replicas equal to maxReplicas, the CPU utilisation against the target and the request,
and the source of the load by name: the Deployment whose pod has the client IP, and its command.
FIX: raise maxReplicas or the CPU request within bounds, or stop the load if it is not legitimate.

## Drift: the live object differs from desired/

    local_list {path: desired}                                            -> the desired manifests
    local_read {path: desired/<file>}                                      -> the desired fields
    get {kind: deployment, name: <name>, namespace: <ns>, raw: true}        -> the live fields

Compare them field by field: replicas, each container's image, resources. Report each field that
differs with its live and its desired value. `diff` is not a tool here: it is a server-side dry run
that this node may not run; the `fix` bot on the write node runs it.
FIX (not applied): apply `desired/<file>`; `fix` proposes it with its diff and rollback.

## The report is the last message

Only the last message of a run reaches the operator. Every message that is not a tool call is the
complete report, `CAUSE:` to `FIX (not applied):`, identical each time. AXLR may add a turn after it:
a note starting `[AXLR · Jev]` (an independent check doubts the answer) or `[AXLR · memory]`.
Neither changes the diagnosis: send the report again, not a sentence about the note.
