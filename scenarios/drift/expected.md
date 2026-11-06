# drift: expected facts

Alert: `LabDeploymentDrift`, namespace `drift-lab`.

The write node's scenario: the judge scores `fix`'s proposal, not triage's diagnosis (see
`needs-write-node`). Every row counts, the proposal and the rollback too.

| Fact | Value |
|---|---|
| object | Deployment `web` |
| namespace | `drift-lab` |
| field 1 | `spec.replicas`: live `3`, desired `2` |
| field 2 | image: live `nginx:1.26-alpine`, desired `nginx:1.27-alpine` |
| evidence | the `diff` tool on `desired/deployment.yaml` answered `differences: true` (a server-side dry run; nothing changed) |
| proposal (not applied) | apply `desired/deployment.yaml` to `drift-lab` (`kubectl apply -f desired/deployment.yaml`) |
| rollback | restore the live state recorded before the apply: 3 replicas and image `nginx:1.26-alpine` |
