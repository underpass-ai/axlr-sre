# oom: expected facts

Alert: `LabPodOOMKilled`, namespace `shop`.

| Fact | Value |
|---|---|
| object | Deployment `cart` (container `cart`) |
| namespace | `shop` |
| reason | `OOMKilled` (exit code 137), restarting |
| limit | `limits.memory: 32Mi` |
| demand | the process loads a 60 MiB cache (log `loading 60 MiB cache`), more than the limit |
| fix (not applied) | raise `limits.memory` above the demand (for example 64Mi–128Mi) or shrink the cache |
