# Runbook of the sre team (phase 0)

Read before the first kubectl call. Every call: `kubectl --context kind-sre-lab <verb> -n <namespace> ...`.
PersistentVolumes, StorageClasses and Nodes are cluster-scoped: no `-n` for them.
Allowed verbs: get, describe, logs, events, top, diff. `rollout` is refused: AXLR asks a decision for it,
and the resumed run's extra turns replace the report. The ReplicaSets show the revisions. Never read Secrets. Change nothing.

## Any alert

get and describe the object the alert names; `get events -n <ns> --sort-by=.lastTimestamp`;
`logs` (with `--previous` after a restart); then what it depends on: ReplicaSets, PVC and PV,
NetworkPolicies, HPA and `top pods`, Certificate and Issuer, the Node's conditions and taints.

## CrashLoopBackOff: what changed

The Deployment keeps one ReplicaSet per revision. Compare them:

    kubectl --context kind-sre-lab get -n <ns> rs -l app=<app> -o custom-columns=NAME:.metadata.name,REVISION:.metadata.annotations.deployment\.kubernetes\.io/revision,CHANGE-CAUSE:.metadata.annotations.kubernetes\.io/change-cause,DESIRED:.spec.replicas,READY:.status.readyReplicas,COMMAND:.spec.template.spec.containers[0].command,IMAGE:.spec.template.spec.containers[0].image

Report: which revision introduced the change (its number and change-cause, e.g. revision 2 "release v2"),
what it changed (command or image, old and new), and that the previous revision ran (its READY was 1
before it was scaled down; the old ReplicaSet's template is the one that ran).
FIX (a write, never run here): roll back to the last revision that ran (`kubectl rollout undo -n <ns> deploy/<name> --to-revision=<n>`)
or ship what the new revision needs.

## Volume full

    kubectl --context kind-sre-lab describe -n <ns> pvc <claim>        -> Capacity, Status, Volume, Used By
    kubectl --context kind-sre-lab describe pv <volume>                -> StorageClass, Capacity, Source, Node Affinity
    kubectl --context kind-sre-lab describe storageclass <class>       -> Provisioner, AllowVolumeExpansion

Report: usage (from the pod's logs or `df`), who writes (the container's command and log lines),
the PV's name, the StorageClass's name and provisioner, and whether the volume can be expanded:
provisioner `kubernetes.io/no-provisioner` with AllowVolumeExpansion unset cannot.
FIX: free space or move to a larger volume; expansion is not possible with such a class.

## Drift

The desired state is the manifests under `desired/` in your workspace, the directory you run in.
Run exactly, with no `-n`, no stdin and no manifest you wrote or edited (they are complete as they are):

    kubectl --context kind-sre-lab diff -f desired/

Exit code 1 means differences, not an error (0 none, 2 a real error). Report the command and
"exit code 1" as one fact, then each field that differs, live (`-` lines) and desired (`+` lines).
FIX: `kubectl apply -f desired/`.

## The report's last line

    Nothing from this incident is recorded in memory, by design.

AXLR asks the model to record memory when an answer does real work and says nothing about memory;
the answer to that question would replace the report. This line answers it in advance. There is no
memory tool in this team: do not look for one. If a console note still follows the report, reply with
the whole report again: the last message is the answer.
