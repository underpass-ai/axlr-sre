# messaging-mcp

SQS and SNS for a bot that fixes queues, through the `aws` CLI: the queues of a region with
their depth, redrive policy and sources, a look at a queue's messages without consuming them,
the topics and their subscriptions; and, with `MESSAGING_WRITES=true`, send, publish, the
native redrive of a dead-letter queue, its cancel, and a drain that saves every message to disk
before deleting it. Every write previews unless `confirm` is true.

```text
bot ──▶ messaging-mcp ──spawns──▶ aws --region R --output json … sqs | sns
                                       sqs list-queues | get-queue-url | get-queue-attributes | list-dead-letter-source-queues
                                       sqs receive-message | send-message-batch | delete-message-batch
                                       sqs start-message-move-task | list-message-move-tasks | cancel-message-move-task
                                       sns list-topics | get-topic-attributes | list-subscriptions-by-topic | publish-batch
                                       cloudwatch get-metric-data (a queue's last day)
                         ──writes──▶ $MESSAGING_SNAPSHOT_DIR/<queue>-<time>.jsonl   (drain's evidence)
```

The runner, the regions, the failure translation and the metric series come from
[`aws-mcp`](../aws/), used as a library: one way to find the CLI, one way to name a region, one
way to read its errors.

## Install and register

```bash
make -C tools install        # builds in release and copies messaging-mcp to ~/.local/bin
```

The `aws` CLI version 2 must be on the node; `doctor` says when it is not.

```json
{
  "command": "/home/<user>/.local/bin/messaging-mcp",
  "env": {"AWS_MCP_REGIONS": "eu-west-1", "AWS_MCP_PROFILE": "ops-rw", "MESSAGING_WRITES": "true"}
}
```

## Configuration

| variable                 | required | meaning                                                                                   |
|--------------------------|----------|-------------------------------------------------------------------------------------------|
| `AWS_MCP_REGIONS`, `AWS_MCP_PROFILE`, `AWS_CLI` | as in `aws-mcp` | the regions allowlist (first is the default; unset: every call names its region), the profile, the executable |
| `MESSAGING_WRITES`       | no       | `true` publishes the five writes; off when unset                                          |
| `MESSAGING_SNAPSHOT_DIR` | no       | where `drain` writes its evidence; `$XDG_STATE_HOME/messaging-mcp/snapshots` or `~/.local/state/messaging-mcp/snapshots` when unset |

No queue, topic, account or region is built in: every call names its queue (URL, or name in the
region) or its topic ARN. A URL or ARN of another region is refused by name.

## Tools

Answers are JSON. A failure is `isError` with `error` and, when there is one, `hint`. Arguments
are validated before `aws` runs; no argument can become a flag. Message bodies appear in answers
(4 KiB each, cut) and, whole, in the evidence file of a drain: the snapshot directory inherits
their sensitivity.

| tool             | arguments                                                                 | answer                                                                                                      |
|------------------|---------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------|
| `doctor`         | —                                                                         | the `aws-mcp` checks (configuration, CLI, credentials), writes on or off, the snapshot directory             |
| `discover`       | `region`?                                                                 | identity, regions, one page of queue names and the topic count per region                                   |
| `queues`         | `prefix`?, `limit` (50, ≤100), `with_depth`, `region`                     | name, url, fifo; with depth, visible/in flight/delayed and whether it has a redrive policy (first fifty; whether it is a dead-letter queue is `queue`'s answer) |
| `queue`          | `queue`, `history` (true), `region`                                       | depth, FIFO, redrive policy (target, max receives), the sources that send failed messages to it, retention, created; the last day of visible messages and the age of the oldest from CloudWatch |
| `peek`           | `queue`, `limit` (10, ≤100), `visibility_seconds` (30), `region`          | messages (id, sent, attempts, group, body) and a summary: unique bodies, groups, attempt counts, oldest     |
| `redrive_status` | `queue`, `region`                                                         | the message move tasks: status, moved, to move, failure                                                     |
| `topics`         | `limit` (50, ≤100), `region`                                              | arn, name, fifo                                                                                             |
| `topic`          | `topic` (ARN), `region`                                                   | display name, FIFO, subscription counts, subscriptions with protocol and endpoint                           |
| `send`           | `queue`, `messages`, `message_group_id`?, `confirm`, `region`             | preview of each message, or `sent`, `failed` per entry, `messageIds`                                        |
| `publish`        | `topic`, `messages`, `message_group_id`?, `confirm`, `region`             | the same                                                                                                    |
| `redrive`        | `queue`, `destination`?, `per_second`?, `confirm`, `region`               | preview with depth and sources, or `taskHandle`                                                             |
| `redrive_cancel` | `queue`, `confirm`, `region`                                              | the running task, or `moved`                                                                                |
| `drain`          | `queue`, `limit` (100, ≤1000), `visibility_seconds` (60), `confirm`, `region` | preview with depth and sources, or `evidence` (the file), `received`, `deleted`, `deleteFailed`, `stoppedBy`, a summary |

**Cost.** Every call is one `aws` process, about half a second of Python start-up each.
`queues` with `with_depth` is one process per queue (fifty queues: about half a minute inside
one tool call); `drain` is two per batch of ten (a thousand messages: two to three minutes);
`peek` with a short `visibility_seconds` stops when a receive brings nothing new.

**`peek` refuses a queue with a redrive policy.** A receive counts as a delivery attempt; on a
source queue it can push messages past `maxReceiveCount` into the dead-letter queue. Look at the
dead-letter queue instead. On any queue the messages stay hidden for `visibility_seconds`.

**Writes.** Published only with `MESSAGING_WRITES=true`; otherwise a call to one is refused with
the way to enable them. Each previews unless `confirm=true`, and a preview reads attributes
only: it never receives, since a receive hides what it returns (tested with doubles).

- `send` and `publish` read the destination first: a FIFO queue or topic needs a group id on every
  message or `message_group_id` as the default, and a deduplication id unless the destination
  deduplicates by content, in which case one is derived from the body. A standard destination
  refuses the FIFO fields. Batches of ten and 256 KiB; `Successful` and `Failed` per entry.
- `redrive` needs a dead-letter queue (one some queue sends to) and no running task; without a
  destination SQS sends each message back to the queue it came from. `redrive_status` follows it.
- `drain` needs a dead-letter queue and no running task. Each batch is received, appended to the
  evidence file and synced, and only then deleted; a failed delete after a save is reported, not
  lost. It stops at `limit`, when the queue is `exhausted`, or `in_flight` when messages remain
  but none can be received (hidden by a visibility timeout, or with a consumer). A failure midway
  (a delete refused, the file not writable) answers `mode: stopped` with the error, the file and
  what was deleted so far, never a bare error. `PurgeQueue` is never used.

**Evidence.** `<dir>/<queue>-<UTC time>.jsonl`, directory 0700, file 0600: a header line (queue,
url, arn, region, started, limit, depth at start) and one line per message with its id, sent
time, attempts, group, attributes and whole body. A torn last line means a crash mid-batch, and
that batch was not deleted.

## IAM

| tools                                        | actions                                                                                           |
|----------------------------------------------|---------------------------------------------------------------------------------------------------|
| reads                                        | `sqs:ListQueues`, `sqs:GetQueueUrl`, `sqs:GetQueueAttributes`, `sqs:ListDeadLetterSourceQueues`, `sqs:ReceiveMessage`, `sqs:ListMessageMoveTasks`, `sns:ListTopics`, `sns:GetTopicAttributes`, `sns:ListSubscriptionsByTopic`, `cloudwatch:GetMetricData`, `sts:GetCallerIdentity` |
| `send`, `publish`                            | `sqs:SendMessage`, `sns:Publish`                                                                  |
| `redrive`, `redrive_cancel`                  | `sqs:StartMessageMoveTask`, `sqs:CancelMessageMoveTask`; the task itself needs receive and delete on the source and send on the destination |
| `drain`                                      | `sqs:ReceiveMessage`, `sqs:DeleteMessage`                                                         |

## Layout

```text
src/
├── bin/messaging_mcp.rs   composition
├── domain/                MessagingConfig, QueueRef, QueueFacts, ReceivedMessage, MessageSummary,
│                          OutboundMessages, Call (the sqs/sns runs), move tasks, topics
├── ports/                 EvidenceStore, EvidenceSink (the runner is aws-mcp's)
├── application/           QueueLookup, InspectQueues, ChangeQueues, DiagnoseSetup, DiscoverScope
└── adapters/
    ├── inbound/           tool catalogue, MessagingMcpHost
    └── outbound/          JsonlEvidence
```

## Verified

With a stateful fake `aws` (Python: receives hide messages, deletes remove them, tasks run and
cancel), 3 November 2026: every tool over stdio; `peek` refused on a source queue; FIFO rules on
`send` and `publish`; `redrive` and `drain` refused while a task runs; `drain` to the limit with
its evidence file (0600, header plus one line per message), then `exhausted`. Not exercised
against a real account: the CLI's answer shapes come from its reference and are pinned by tests.

Out of scope here: queue catalogues and aliases, tenancy tags, approval tokens with a phrase
(replaced by `MESSAGING_WRITES` plus `confirm=true`), and large payloads through S3.
