# aws-mcp

AWS, read only, for a cloud diagnosis bot, through the `aws` CLI: CloudWatch metrics and
alarms, Lambda functions with what runs (alias), where they log (log group) and what triggers
them, and the log events themselves. Every call names its region, checked against the server's
allowlist; every answer is bounded; nothing writes.

```text
bot ──▶ aws-mcp ──spawns──▶ aws --region R --output json --no-cli-pager [--profile P] <service> <operation> …
                                 cloudwatch get-metric-data | describe-alarms
                                 lambda list-functions | get-function-configuration | list-aliases | list-event-source-mappings | get-policy
                                 logs filter-log-events | describe-log-groups
                                 sts get-caller-identity
```

The CLI, not the SDK: the node image already carries tooling, the CLI resolves every credential
source the same way for everyone (a role on the node through IRSA, an SSO session, a profile),
and a tool that is a subprocess is tested with a fake executable, as `kube-mcp` is with
`kubectl`. Swapping the runner for the SDK later is one adapter behind one port.

## Install and register

```bash
make -C tools install        # builds in release and copies aws-mcp to ~/.local/bin
```

The `aws` CLI **version 2** must be on the node; `doctor` says when it is not.

```json
{
  "command": "/home/<user>/.local/bin/aws-mcp",
  "env": {"AWS_MCP_REGIONS": "eu-west-1,us-east-1", "AWS_MCP_PROFILE": "ops-ro"}
}
```

## Configuration

| variable          | required | meaning                                                                                              |
|-------------------|----------|------------------------------------------------------------------------------------------------------|
| `AWS_MCP_REGIONS` | no       | comma-separated allowlist; the first is the default. Unset: every call names its region             |
| `AWS_MCP_PROFILE` | no       | passed as `--profile`. Unset: the CLI's own credential chain, which is how a node with a role works  |
| `AWS_CLI`         | no       | the executable; `aws` on the PATH when unset                                                         |

**No ambient default.** `--region` is passed on every run; a call that names no region with
`AWS_MCP_REGIONS` unset is refused, so an `AWS_DEFAULT_REGION` in the node's environment never
decides where a bot looks. Nothing else of the environment is touched (only `AWS_PAGER` is
emptied): IRSA and SSO live there. No account, region or profile is built in.

## Tools

Answers are JSON. A failure is `isError` with `error` and, when there is one, `hint`. Arguments
are validated before `aws` runs; no argument can become a flag. Windows take RFC 3339 or how
long ago (`15m`, `3h`, `1d`), fourteen days at most; times in answers are UTC with their `Z`.

| tool               | arguments                                                              | answer                                                                                                             |
|--------------------|------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------|
| `doctor`           | —                                                                      | the configuration, whether `aws` v2 runs, who the credentials are (`sts get-caller-identity`: account and ARN), an action for what fails |
| `discover`         | `region`?                                                              | the identity, the regions in scope and, for the first five, alarms in ALARM and one page of functions              |
| `metric_query`     | `namespace`, `metric`, `dimensions`, `stat` (Sum), `period` (300), `start` (3h), `end`, `region` | the series compacted, `points`, `last`                                                                   |
| `lambda_health`    | `function`, `period`, `start` (3h), `end`, `region`                    | invocations, errors, throttles, duration average and p99, concurrency, each compacted; `warning` when there is not one point |
| `alarms`           | `state`?, `prefix`?, `limit` (50, ≤100), `region`                      | name, kind, state, since, reason (200 chars), what it watches; `inAlarm`, `more`                                   |
| `lambda_functions` | `contains`?, `limit` (50, ≤100), `region`                              | name, runtime, memory, timeout, state, last modified, log group and whether it diverges from `/aws/lambda/<name>` |
| `lambda_function`  | `function`, `region`                                                   | the above plus aliases with their versions, triggers, handler, layers. **No environment variables**                |
| `logs_filter`      | `log_group` or `function`, `filter`?, `start` (30m), `end`, `limit` (200, ≤1000), `region` | events oldest first, 4 KiB per message and 256 KiB at most, `truncated`; a warning when the group was derived and empty |
| `logs_groups`      | `prefix`?, `limit` (50, ≤100), `region`                                | name, stored bytes, retention                                                                                      |

**Series.** Three shapes, and confusing them leads to opposite conclusions: `no data` (CloudWatch
has not one point: the function was not invoked, or the name is not exact; **not a failure**),
`all zero (N points)` (there were points and every one is zero: no errors), or the points where
the value changes plus the last one, `[time, value]`, 200 at most. `metric_query` keeps zeros,
since the caller chose the metric. `get-metric-data` is asked oldest first and the domain sorts
anyway.

**Paging.** Every listing runs with `--max-items` and walks the CLI's `NextToken` itself, twenty
pages at most, so a rare filter on a large log group never fetches to exhaustion; `more` and
`truncated` say when there was more.

**Failures.** The CLI's stderr is translated: an expired SSO session says `aws sso login
--profile <p>`; no credentials or an unknown profile name `AWS_MCP_PROFILE`; `AccessDenied*`
points at `doctor`; `Throttling*` asks for less; `ResourceNotFound*` asks for the exact name and
region. The raw dump never travels.

**Secrets.** Function environment variables are never read into an answer: `lambda_functions`
and `lambda_function` pick their fields. The identity shown by `doctor` is the account id and the
role ARN the operator configured.

## Layout

```text
src/
├── bin/aws_mcp.rs      composition
├── domain/             ServerConfig, Regions, tokens, TimeWindow, Series, Invocation, Failure,
│                       the alarm, function, trigger and log parsers
├── ports/              AwsRunner, Completed
├── application/        QueryMetrics, InspectLambda, QueryLogs, DiagnoseSetup, DiscoverScope
└── adapters/
    ├── inbound/        tool catalogue, AwsMcpHost
    └── outbound/       ProcessRunner (spawns aws)
```

## Verified

With a fake `aws` (Python, answering realistic JSON and honouring `--max-items` and
`--starting-token`), 3 November 2026: every tool over stdio, paging, a missing function, a region
outside the allowlist, no secret in any answer. With the real `aws` 2.36 and an empty
configuration: `doctor` reports `aws-cli/2`, an unknown profile and no credentials, each with its
action; no account was reached. Not exercised against a real account: the answer shapes come from
the API reference and the CLI's documentation and are pinned by tests.

Out of scope here: SQS and dead-letter queues (next tool), writes (the remediation role of the
design is phase 3), and the EKS, EC2, RDS and ELB describes of `diag-cloud`, which are a tool of
their own.
