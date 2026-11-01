# netpol: expected facts

Alert: `LabPodNotReady`, namespace `shop`, pod `payments-…`.

| Fact | Value |
|---|---|
| object | Deployment `payments`, running but not ready |
| namespace | `shop` |
| symptom | readiness probe `nslookup kubernetes.default.svc.cluster.local` fails (timeout / `can't resolve`) |
| cause | NetworkPolicy `deny-all` (`podSelector: {}`, `policyTypes: [Egress]`, no egress rule) blocks DNS, 53/UDP to kube-dns |
| fix (not applied) | allow egress to kube-dns on 53/UDP and 53/TCP, or remove `deny-all` |
