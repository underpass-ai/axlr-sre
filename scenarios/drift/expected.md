# drift: expected facts

Alert: `LabDeploymentDrift`, namespace `drift-lab`.

| Fact | Value |
|---|---|
| object | Deployment `web` |
| namespace | `drift-lab` |
| field 1 | `spec.replicas`: live `3`, desired `2` |
| field 2 | image: live `nginx:1.26-alpine`, desired `nginx:1.27-alpine` |
| evidence | `kubectl diff -f desired/` exit 1 (differences, not an error) |
| fix (not applied) | `kubectl apply -f desired/` |
