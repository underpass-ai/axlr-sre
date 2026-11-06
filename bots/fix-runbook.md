# Runbook of fix (phase 1, propose)

`register.sh` copies this file into fix's workspace as `runbook.md`. Read it before the first
cluster call.

You propose a change and how to undo it; you apply nothing. The cluster is reached only through
the `kube` server's tools on the write node, as the ServiceAccount `axlr-sre-write`: `discover`,
`get`, `describe`, `events`, `rollout_history` and `diff`. Call them with `axlr_call_tool` by that
exact name: `diff`, never `kube/diff`. Every call names the incident's `namespace`. `diff` is a
server-side dry run: it shows what an apply would change and changes nothing. There is no apply,
scale or rollout tool on this node: a proposal is words, never a call.

## Drift: the live object differs from desired/

    local_list {path: desired}                                            -> the desired manifests
    local_read {path: desired/<file>}                                      -> the manifest's text
    diff {manifest: <that text, as it is>, namespace: <ns>}                -> differences and the diff
    get {kind: deployment, name: <name>, namespace: <ns>, raw: true}        -> the live object

`differences: true` is drift, not an error; `false` means there is nothing to propose: say so.
In the diff, `-` lines are the live state and `+` lines the desired one. Record the live values
before proposing: they are the rollback.

## The report

Write it after your last tool call, in this shape only:

    CAUSE: <object, namespace, the drift in one sentence>
    DIFF: where it comes from, first: "diff tool on desired/<file>, server-side dry run:
          differences: true" (or, if diff failed, its error and "compared by hand");
          then each field that differs, live -> desired
    PROPOSAL (not applied): kubectl apply -n <ns> -f desired/<file>
    ROLLBACK: the live values recorded now (replicas, image...) and the commands that restore them,
              e.g. kubectl -n <ns> scale deploy/<name> --replicas=<live> and
              kubectl -n <ns> set image deploy/<name> <container>=<live image>
    CHECK: diff of the same manifest answers differences: false; the rollout completes

Only the last message of a run reaches the operator. Every message that is not a tool call is the
complete report, identical each time. A note starting `[AXLR · Jev]` or `[AXLR · memory]` changes
nothing: send the report again, not a sentence about the note.
