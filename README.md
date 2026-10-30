# AXLR Bots SRE

A brigade of bots that resolves Kubernetes and cloud incidents on [AXLR Bots](https://github.com/underpass-ai/axlr-bots): it receives the alert, diagnoses with evidence, proposes the fix as a diff, applies it when a person approves, verifies and writes the postmortem. The model proposes; the policy and the person decide; the panel shows everything.

**Status:** design, revision 2 (30 October 2026). Nothing runs yet. The design is [docs/design.md](docs/design.md) (Spanish).

```text
Alertmanager ──webhook──▶ axlr-bots control plane ──▶ Incident ──▶ triage ──▶ diag-k8s / diag-cloud ──▶ fix (approval) ──▶ verify ──▶ scribe
```

## What lives where

| Piece | Repo |
|---|---|
| Incident aggregate, `alertmanager` trigger, approvers, per-incident budget | [axlr-bots](https://github.com/underpass-ai/axlr-bots) (the control plane) |
| the bots' definitions, the runbooks as signed skills, the `kube` tool server, the kind lab with its scenarios, the judge, the Helm chart | this repo |

## Layout (planned)

```text
docs/design.md      the design and its decisions
bots/               triage, diag-k8s, diag-cloud, fix, verify, scribe
runbooks/           crashloop, oom, image-pull, … as catalog skills
tools/kube/         one MCP tool per kubectl verb
lab/                kind + kube-prometheus-stack, 10 fault scenarios
judge/              runs the scenarios, judges the diagnoses, prints the table
deploy/             Helm: two node pods (read-only, write) with their ServiceAccounts
```

## License

Apache License 2.0; see [LICENSE](LICENSE). Part of [Underpass AI](https://underpassai.com).
