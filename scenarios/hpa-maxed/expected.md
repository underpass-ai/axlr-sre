# hpa-maxed: expected facts

Alert: `LabHPAMaxedOut`, namespace `shop`, label `horizontalpodautoscaler=search`.

| Fact | Value |
|---|---|
| object | HPA `search` on Deployment `search` |
| namespace | `shop` |
| state | current replicas `3` = `maxReplicas` `3`; `ScalingLimited` / `TooManyReplicas` |
| load | CPU above the `50 %` target (request `10m`) |
| source | the load comes from the Deployment `load`, which calls `search` |
| fix (not applied) | raise `maxReplicas` within bounds, or the CPU request; or stop the load if it is not legitimate |
