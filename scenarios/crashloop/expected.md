# crashloop: expected facts

Alert: `LabPodCrashLooping`, namespace `shop`.

A diagnosis names the cause when it states these facts (wording is free):

| Fact | Value |
|---|---|
| object | Deployment `checkout` (its pod `checkout-…`) |
| namespace | `shop` |
| state | `CrashLoopBackOff` |
| exit code | `1` |
| log line | `config file /etc/checkout/config.yaml not found` |
| what changed | revision 2 (`release v2`) changed the command; revision 1 ran |
| fix (not applied) | roll back to revision 1 (`kubectl rollout undo deploy/checkout`) or ship the config |
