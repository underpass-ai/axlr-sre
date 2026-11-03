use std::sync::Arc;

use anyhow::{Result, bail};
use aws_mcp::application::shared::run_json;
use aws_mcp::domain::Regions;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::QueueLookup;
use crate::domain::{
    Call, InputError, MessageSummary, OutboundMessages, QueueFacts, RECEIVE_MAX, ReceivedMessage,
    move_tasks, received_messages, topic_facts, visibility,
};
use crate::ports::EvidenceStore;

const NEXT: &str = "call again with confirm=true to run it";
const DRAIN_MAX: i64 = 1_000;
const EMPTY_RECEIVES_MAX: u32 = 3;

/// The writes: `send`, `publish`, `redrive`, `redrive_cancel`, `drain`.
/// Every one previews unless `confirm` is true; a preview reads attributes
/// only and never receives, since a receive hides what it returns. The
/// writes must be on, else nothing runs.
pub struct ChangeQueues {
    lookup: Arc<QueueLookup>,
    regions: Regions,
    writes: bool,
    evidence: Arc<dyn EvidenceStore>,
}

impl ChangeQueues {
    pub fn new(
        lookup: Arc<QueueLookup>,
        regions: Regions,
        writes: bool,
        evidence: Arc<dyn EvidenceStore>,
    ) -> Self {
        Self {
            lookup,
            regions,
            writes,
            evidence,
        }
    }

    fn require_writes(&self) -> Result<()> {
        if self.writes {
            Ok(())
        } else {
            bail!(InputError::WritesDisabled)
        }
    }

    /// Messages into a queue, in batches of ten. The queue's FIFO settings
    /// decide what each message needs.
    pub async fn send(
        &self,
        region: Option<&str>,
        queue: &str,
        messages: &Value,
        default_group_id: Option<&str>,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let region = self.regions.resolve(region)?;
        let facts = self.lookup.resolve(&region, queue).await?;
        let outbound = OutboundMessages::parse(
            messages,
            default_group_id,
            facts.fifo,
            facts.content_based_deduplication,
        )?;
        let batches = outbound.batches();
        let mut answer = json!({
            "mode": "preview",
            "action": "send",
            "region": region,
            "queue": facts.name,
            "url": facts.url,
            "fifo": facts.fifo,
            "count": outbound.len(),
            "batches": batches.len(),
            "messages": outbound.preview(),
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let mut sent = Vec::new();
        let mut failed = Vec::new();
        for batch in &batches {
            let result = run_json(
                self.lookup.runner().as_ref(),
                &Call::send_batch(&region, &facts.url, &OutboundMessages::sqs_entries(batch))?,
                self.lookup.profile(),
            )
            .await?;
            collect_outcomes(&result, &mut sent, &mut failed);
        }
        done(&mut answer, sent, failed);
        Ok(answer)
    }

    /// Messages to a topic, in batches of ten.
    pub async fn publish(
        &self,
        region: Option<&str>,
        topic_arn: &str,
        messages: &Value,
        default_group_id: Option<&str>,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let region = self.regions.resolve(region)?;
        let attributes = run_json(
            self.lookup.runner().as_ref(),
            &Call::topic_attributes(&region, topic_arn)?,
            self.lookup.profile(),
        )
        .await?;
        let topic = topic_facts(topic_arn.trim(), &attributes);
        let outbound = OutboundMessages::parse(
            messages,
            default_group_id,
            topic["fifo"] == true,
            topic["contentBasedDeduplication"] == true,
        )?;
        let batches = outbound.batches();
        let mut answer = json!({
            "mode": "preview",
            "action": "publish",
            "region": region,
            "topic": topic["name"],
            "arn": topic["arn"],
            "fifo": topic["fifo"],
            "subscriptionsConfirmed": topic["subscriptionsConfirmed"],
            "count": outbound.len(),
            "batches": batches.len(),
            "messages": outbound.preview(),
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let mut sent = Vec::new();
        let mut failed = Vec::new();
        for batch in &batches {
            let result = run_json(
                self.lookup.runner().as_ref(),
                &Call::publish_batch(&region, topic_arn, &OutboundMessages::sns_entries(batch))?,
                self.lookup.profile(),
            )
            .await?;
            collect_outcomes(&result, &mut sent, &mut failed);
        }
        done(&mut answer, sent, failed);
        Ok(answer)
    }

    /// The native redrive of a dead-letter queue: every message back to
    /// the queue it came from, or all of them to `destination`. One task
    /// at a time per queue.
    pub async fn redrive(
        &self,
        region: Option<&str>,
        queue: &str,
        destination: Option<&str>,
        per_second: Option<i64>,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let region = self.regions.resolve(region)?;
        let facts = self.dead_letter(&region, queue).await?;
        let destination_arn = match destination.map(str::trim).filter(|d| !d.is_empty()) {
            Some(raw) => Some(self.lookup.resolve(&region, raw).await?.arn),
            None => None,
        };
        let invocation =
            Call::start_move(&region, &facts.arn, destination_arn.as_deref(), per_second)?;
        self.refuse_while_running(&region, &facts).await?;
        let mut answer = json!({
            "mode": "preview",
            "action": "redrive",
            "region": region,
            "queue": facts.name,
            "depth": facts.depth.to_json(),
            "sources": facts.sources,
            "destination": destination_arn.clone().unwrap_or_else(|| "each message back to the queue it came from".into()),
            "perSecond": per_second,
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let started = run_json(
            self.lookup.runner().as_ref(),
            &invocation,
            self.lookup.profile(),
        )
        .await?;
        answer["mode"] = json!("done");
        answer["taskHandle"] = started["TaskHandle"].clone();
        answer["hint"] = json!("redrive_status shows the task's progress");
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    /// Cancels the running move task of a queue.
    pub async fn redrive_cancel(
        &self,
        region: Option<&str>,
        queue: &str,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let region = self.regions.resolve(region)?;
        let facts = self.lookup.resolve(&region, queue).await?;
        let listing = run_json(
            self.lookup.runner().as_ref(),
            &Call::list_move_tasks(&region, &facts.arn)?,
            self.lookup.profile(),
        )
        .await?;
        let tasks = move_tasks(&listing);
        let Some(running) = tasks.iter().find(|t| t.is_running()) else {
            bail!("no message move task is running on {}", facts.name);
        };
        let Some(handle) = running.handle.as_deref() else {
            bail!("the running task has no handle to cancel");
        };
        let invocation = Call::cancel_move(&region, handle)?;
        let mut answer = json!({
            "mode": "preview",
            "action": "redrive_cancel",
            "region": region,
            "queue": facts.name,
            "task": running.to_json(),
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let cancelled = run_json(
            self.lookup.runner().as_ref(),
            &invocation,
            self.lookup.profile(),
        )
        .await?;
        answer["mode"] = json!("done");
        answer["moved"] = cancelled["ApproximateNumberOfMessagesMoved"].clone();
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    /// Empties a dead-letter queue keeping every message: a batch is
    /// received, written to the evidence file and made durable, and only
    /// then deleted from the queue. If anything fails midway, what was
    /// deleted is in the file. `PurgeQueue` is never used.
    pub async fn drain(
        &self,
        region: Option<&str>,
        queue: &str,
        limit: i64,
        visibility_seconds: i64,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let region = self.regions.resolve(region)?;
        if !(1..=DRAIN_MAX).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: DRAIN_MAX
            });
        }
        let visibility_seconds = visibility(visibility_seconds)?;
        let facts = self.dead_letter(&region, queue).await?;
        self.refuse_while_running(&region, &facts).await?;
        let mut answer = json!({
            "mode": "preview",
            "action": "drain",
            "region": region,
            "queue": facts.name,
            "url": facts.url,
            "depth": facts.depth.to_json(),
            "sources": facts.sources,
            "limit": limit,
            "evidence": "a JSON Lines file under the snapshot directory, one line per message, written before each delete",
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let header = json!({
            "queue": facts.name,
            "url": facts.url,
            "arn": facts.arn,
            "region": region,
            "started": Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "limit": limit,
            "depthAtStart": facts.depth.to_json(),
        });
        let mut sink = self.evidence.start(&facts.name, &header)?;
        let mut kept: Vec<ReceivedMessage> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut deleted = 0usize;
        let mut delete_failed: Vec<Value> = Vec::new();
        let mut empty_receives = 0;
        let mut failure: Option<String> = None;
        let stopped_by = loop {
            let remaining = limit - kept.len() as i64;
            if remaining <= 0 {
                break "limit";
            }
            let receive = Call::receive(
                &region,
                &facts.url,
                remaining.min(RECEIVE_MAX),
                visibility_seconds,
            )?;
            let answer = match run_json(
                self.lookup.runner().as_ref(),
                &receive,
                self.lookup.profile(),
            )
            .await
            {
                Ok(answer) => answer,
                Err(error) => {
                    failure = Some(format!("{error:#}"));
                    break "failure";
                }
            };
            // A message a failed delete left behind can come back: it is kept once.
            let received: Vec<ReceivedMessage> = received_messages(&answer)
                .into_iter()
                .filter(|m| seen.insert(m.id.clone()))
                .collect();
            if received.is_empty() {
                empty_receives += 1;
                if empty_receives < EMPTY_RECEIVES_MAX {
                    continue;
                }
                let now = match self.lookup.attributes(&region, &facts.url).await {
                    Ok(now) => now,
                    Err(error) => {
                        failure = Some(format!("{error:#}"));
                        break "failure";
                    }
                };
                break if now.depth.visible > 0 || now.depth.in_flight > 0 {
                    "in_flight"
                } else {
                    "exhausted"
                };
            }
            empty_receives = 0;
            let lines: Vec<Value> = received
                .iter()
                .map(ReceivedMessage::evidence_line)
                .collect();
            if let Err(error) = sink.append(&lines) {
                failure = Some(format!("{error:#}; the batch was not deleted"));
                break "failure";
            }
            let entries = Value::Array(
                received
                    .iter()
                    .enumerate()
                    .map(|(i, m)| json!({"Id": i.to_string(), "ReceiptHandle": m.receipt_handle}))
                    .collect(),
            );
            kept.extend(received);
            let delete = Call::delete_batch(&region, &facts.url, &entries)?;
            match run_json(
                self.lookup.runner().as_ref(),
                &delete,
                self.lookup.profile(),
            )
            .await
            {
                Ok(result) => {
                    deleted += result["Successful"].as_array().map_or(0, Vec::len);
                    if let Some(failed) = result["Failed"].as_array() {
                        delete_failed.extend(failed.iter().map(
                            |f| json!({"id": f["Id"], "code": f["Code"], "message": f["Message"]}),
                        ));
                    }
                }
                Err(error) => {
                    failure = Some(format!(
                        "{error:#}; the batch is in the evidence file and still in the queue"
                    ));
                    break "failure";
                }
            }
        };
        // From here on the answer is a report, never a bare error: what was
        // deleted is in the file, and the operator must learn where.
        answer["mode"] = json!(if failure.is_some() { "stopped" } else { "done" });
        answer["error"] = json!(failure);
        answer["evidence"] = json!(sink.path().display().to_string());
        answer["received"] = json!(kept.len());
        answer["deleted"] = json!(deleted);
        answer["deleteFailed"] = json!(delete_failed);
        answer["stoppedBy"] = json!(stopped_by);
        if stopped_by == "in_flight" {
            answer["hint"] = json!(format!(
                "messages remain but none could be received: hidden by a visibility timeout ({} s here) or in flight with a consumer; call again later",
                visibility_seconds
            ));
        }
        answer["summary"] = MessageSummary::of(&kept).to_json();
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    async fn dead_letter(&self, region: &str, queue: &str) -> Result<QueueFacts> {
        let facts = self.lookup.resolve(region, queue).await?;
        if !facts.is_dead_letter() {
            bail!(InputError::NotDeadLetter(facts.name));
        }
        Ok(facts)
    }

    async fn refuse_while_running(&self, region: &str, facts: &QueueFacts) -> Result<()> {
        let listing = run_json(
            self.lookup.runner().as_ref(),
            &Call::list_move_tasks(region, &facts.arn)?,
            self.lookup.profile(),
        )
        .await?;
        if let Some(task) = move_tasks(&listing).into_iter().find(|t| t.is_running()) {
            bail!(InputError::TaskRunning {
                status: task.status,
                handle: task.handle.unwrap_or_default()
            });
        }
        Ok(())
    }
}

fn collect_outcomes(result: &Value, sent: &mut Vec<Value>, failed: &mut Vec<Value>) {
    if let Some(ok) = result["Successful"].as_array() {
        sent.extend(
            ok.iter()
                .map(|e| json!({"index": e["Id"], "messageId": e["MessageId"]})),
        );
    }
    if let Some(bad) = result["Failed"].as_array() {
        failed.extend(bad.iter().map(|e| json!({"index": e["Id"], "code": e["Code"], "message": e["Message"], "senderFault": e["SenderFault"]})));
    }
}

fn done(answer: &mut Value, sent: Vec<Value>, failed: Vec<Value>) {
    answer["mode"] = json!("done");
    answer["sent"] = json!(sent.len());
    answer["failed"] = json!(failed);
    answer["messageIds"] = json!(sent);
    answer.as_object_mut().map(|o| o.remove("messages"));
    answer.as_object_mut().map(|o| o.remove("next"));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::super::test_doubles::{
        ARN, FakeEvidence, FakeRunner, Recording, SOURCE, TOPIC, URL, dlq_attributes, message,
    };
    use super::*;

    fn change(
        runner: Arc<dyn aws_mcp::ports::AwsRunner>,
        evidence: Arc<FakeEvidence>,
        writes: bool,
    ) -> ChangeQueues {
        ChangeQueues::new(
            Arc::new(QueueLookup::new(runner, None)),
            Regions::new(Some("eu-west-1")).unwrap(),
            writes,
            evidence,
        )
    }

    fn dlq() -> Vec<(&'static str, Result<Value, String>)> {
        vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            ("sqs list-message-move-tasks", Ok(json!({"Results": []}))),
        ]
    }

    #[tokio::test]
    async fn send_previews_then_sends_in_batches_and_reports_failures() {
        let mut answers = dlq();
        answers.push(("sqs send-message-batch", Ok(json!({"Successful": [{"Id": "0", "MessageId": "a"}], "Failed": [{"Id": "1", "Code": "X", "Message": "bad", "SenderFault": true}]}))));
        let runner = FakeRunner::answering(answers);
        let change = change(runner.clone(), Arc::new(FakeEvidence::default()), true);
        let messages = json!([{"body": "one"}, {"body": {"n": 2}}]);
        let preview = change
            .send(None, URL, &messages, None, false)
            .await
            .unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["count"], 2);
        assert_eq!(preview["messages"][1]["body"], "{\"n\":2}");
        assert!(runner.writes().is_empty());
        let done = change.send(None, URL, &messages, None, true).await.unwrap();
        assert_eq!(done["mode"], "done");
        assert_eq!(done["sent"], 1);
        assert_eq!(done["failed"][0]["code"], "X");
        assert!(done.get("messages").is_none());
        assert_eq!(runner.writes(), ["sqs send-message-batch"]);
        assert!(
            change
                .send(
                    None,
                    URL,
                    &json!([{"body": "x", "message_group_id": "g"}]),
                    None,
                    true
                )
                .await
                .is_err()
        );
        assert_eq!(runner.writes().len(), 1);
    }

    #[tokio::test]
    async fn publish_reads_the_topic_first() {
        let runner = FakeRunner::answering(vec![
            (
                "sns get-topic-attributes",
                Ok(
                    json!({"Attributes": {"FifoTopic": "true", "ContentBasedDeduplication": "true", "SubscriptionsConfirmed": "2"}}),
                ),
            ),
            (
                "sns publish-batch",
                Ok(json!({"Successful": [{"Id": "0", "MessageId": "m"}], "Failed": []})),
            ),
        ]);
        let change = change(runner.clone(), Arc::new(FakeEvidence::default()), true);
        let messages = json!([{"body": "evt"}]);
        assert!(
            change
                .publish(None, TOPIC, &messages, None, false)
                .await
                .is_err(),
            "a FIFO topic needs a group"
        );
        let preview = change
            .publish(None, TOPIC, &messages, Some("g1"), false)
            .await
            .unwrap();
        assert_eq!(preview["fifo"], true);
        assert_eq!(preview["messages"][0]["groupId"], "g1");
        assert!(runner.writes().is_empty());
        let done = change
            .publish(None, TOPIC, &messages, Some("g1"), true)
            .await
            .unwrap();
        assert_eq!(done["sent"], 1);
        assert_eq!(done["messageIds"][0]["messageId"], "m");
        assert!(runner.lines().last().unwrap().contains("MessageGroupId"));
    }

    #[tokio::test]
    async fn redrive_needs_a_dead_letter_queue_without_a_running_task() {
        let mut answers = dlq();
        answers.push((
            "sqs start-message-move-task",
            Ok(json!({"TaskHandle": "h-1"})),
        ));
        let runner = FakeRunner::answering(answers);
        let change = change(runner.clone(), Arc::new(FakeEvidence::default()), true);
        let preview = change
            .redrive(None, URL, None, Some(10), false)
            .await
            .unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(
            preview["destination"],
            "each message back to the queue it came from"
        );
        assert_eq!(preview["sources"][0], SOURCE);
        assert!(runner.writes().is_empty());
        let done = change
            .redrive(None, URL, None, Some(10), true)
            .await
            .unwrap();
        assert_eq!(done["taskHandle"], "h-1");
        assert!(runner.lines().last().unwrap().contains(&format!(
            "--source-arn {ARN} --max-number-of-messages-per-second 10"
        )));
        let explicit = change
            .redrive(None, URL, Some(SOURCE), None, false)
            .await
            .unwrap();
        assert_eq!(
            explicit["destination"], ARN,
            "the fake answers the same attributes for every queue"
        );

        runner.set(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": []})),
            ),
        ]);
        let error = change
            .redrive(None, URL, None, None, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a dead-letter queue"), "{error}");
        let mut running = dlq();
        running[2] = (
            "sqs list-message-move-tasks",
            Ok(json!({"Results": [{"TaskHandle": "h-0", "Status": "RUNNING"}]})),
        );
        runner.set(running);
        let error = change
            .redrive(None, URL, None, None, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("already RUNNING") && error.contains("h-0"),
            "{error}"
        );
        assert_eq!(runner.writes(), ["sqs start-message-move-task"]);
    }

    #[tokio::test]
    async fn redrive_cancel_finds_the_running_task() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "sqs list-message-move-tasks",
                Ok(
                    json!({"Results": [{"TaskHandle": "h-0", "Status": "RUNNING"}, {"Status": "COMPLETED"}]}),
                ),
            ),
            (
                "sqs cancel-message-move-task",
                Ok(json!({"ApproximateNumberOfMessagesMoved": 4})),
            ),
        ]);
        let change = change(runner.clone(), Arc::new(FakeEvidence::default()), true);
        let preview = change.redrive_cancel(None, URL, false).await.unwrap();
        assert_eq!(preview["task"]["handle"], "h-0");
        assert!(runner.writes().is_empty());
        let done = change.redrive_cancel(None, URL, true).await.unwrap();
        assert_eq!(done["moved"], 4);
        assert!(runner.lines().last().unwrap().contains("--task-handle h-0"));
        runner.set(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": []})),
            ),
            (
                "sqs list-message-move-tasks",
                Ok(json!({"Results": [{"Status": "RUNNING"}]})),
            ),
        ]);
        assert!(
            change
                .redrive_cancel(None, URL, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("no handle")
        );
        runner.set(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": []})),
            ),
            ("sqs list-message-move-tasks", Ok(json!({"Results": []}))),
        ]);
        assert!(
            change
                .redrive_cancel(None, URL, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("no message move task is running")
        );
    }

    #[tokio::test]
    async fn drain_saves_each_batch_before_deleting_it() {
        let mut answers = dlq();
        answers.push((
            "once:sqs receive-message",
            Ok(json!({"Messages": [message("m1", "a"), message("m2", "b")]})),
        ));
        answers.push((
            "once:sqs receive-message",
            Ok(json!({"Messages": [message("m3", "a")]})),
        ));
        answers.push(("sqs receive-message", Ok(Value::Null)));
        answers.push((
            "sqs delete-message-batch",
            Ok(json!({"Successful": [{"Id": "0"}, {"Id": "1"}], "Failed": []})),
        ));
        let inner = FakeRunner::answering(answers);
        let events = Arc::new(Mutex::new(Vec::new()));
        let recording: Arc<dyn aws_mcp::ports::AwsRunner> = Arc::new(Recording {
            inner: inner.clone(),
            events: events.clone(),
        });
        let evidence = Arc::new(FakeEvidence {
            events: events.clone(),
            ..FakeEvidence::default()
        });
        let change = change(recording, evidence.clone(), true);
        let preview = change.drain(None, URL, 10, 30, false).await.unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["depth"]["visible"], 3);
        assert!(
            !inner.verbs().iter().any(|v| v == "sqs receive-message"),
            "a preview never receives"
        );
        assert!(inner.writes().is_empty());

        inner.set(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(0))),
            ("sqs list-dead-letter-source-queues", Ok(json!({"queueUrls": [SOURCE]}))),
            ("sqs list-message-move-tasks", Ok(json!({"Results": []}))),
            ("once:sqs receive-message", Ok(json!({"Messages": [message("m1", "a"), message("m2", "b")]}))),
            ("once:sqs receive-message", Ok(json!({"Messages": [message("m3", "a")]}))),
            ("sqs receive-message", Ok(Value::Null)),
            ("sqs delete-message-batch", Ok(json!({"Successful": [{"Id": "0"}, {"Id": "1"}], "Failed": [{"Id": "9", "Code": "ReceiptHandleIsInvalid", "Message": "x"}]}))),
        ]);
        let done = change.drain(None, URL, 10, 30, true).await.unwrap();
        assert_eq!(done["mode"], "done");
        assert_eq!(done["received"], 3);
        assert_eq!(
            done["deleted"], 4,
            "the fake answers two successes per delete"
        );
        assert_eq!(done["deleteFailed"][0]["code"], "ReceiptHandleIsInvalid");
        assert_eq!(done["stoppedBy"], "exhausted");
        assert_eq!(done["evidence"], "/evidence/demo-orders-dlq.jsonl");
        assert_eq!(done["summary"]["uniqueBodies"], 2);
        assert_eq!(
            *events.lock().unwrap(),
            ["save:2", "delete", "save:1", "delete"]
        );
        let lines = evidence.lines.lock().unwrap();
        assert_eq!(lines.len(), 4, "a header and three messages");
        assert_eq!(lines[0]["queue"], "demo-orders-dlq");
        assert_eq!(lines[1]["body"], "a");
        assert!(lines[1].get("receiptHandle").is_none());
    }

    #[tokio::test]
    async fn drain_stops_at_the_limit_or_when_messages_stay_hidden() {
        let mut answers = dlq();
        answers.push((
            "once:sqs receive-message",
            Ok(json!({"Messages": [message("m1", "a")]})),
        ));
        answers.push((
            "sqs receive-message",
            Ok(json!({"Messages": [message("m2", "a")]})),
        ));
        answers.push((
            "sqs delete-message-batch",
            Ok(json!({"Successful": [{"Id": "0"}]})),
        ));
        let runner = FakeRunner::answering(answers);
        let change = change(runner.clone(), Arc::new(FakeEvidence::default()), true);
        let limited = change.drain(None, URL, 2, 30, true).await.unwrap();
        assert_eq!(limited["stoppedBy"], "limit");
        assert_eq!(limited["received"], 2);
        assert!(
            runner
                .lines()
                .iter()
                .any(|l| l.contains("--max-number-of-messages 1"))
        );

        let mut answers = dlq();
        answers.push(("sqs receive-message", Ok(Value::Null)));
        runner.set(answers);
        let hidden = change.drain(None, URL, 10, 30, true).await.unwrap();
        assert_eq!(hidden["stoppedBy"], "in_flight");
        assert!(hidden["hint"].as_str().unwrap().contains("visibility"));
        assert_eq!(hidden["deleted"], 0);
    }

    #[tokio::test]
    async fn a_failure_midway_reports_what_was_deleted_and_where_the_evidence_is() {
        let mut answers = dlq();
        answers.push((
            "once:sqs receive-message",
            Ok(json!({"Messages": [message("m1", "a")]})),
        ));
        answers.push((
            "once:sqs receive-message",
            Ok(json!({"Messages": [message("m2", "b")]})),
        ));
        answers.push((
            "once:sqs delete-message-batch",
            Ok(json!({"Successful": [{"Id": "0"}]})),
        ));
        answers.push(("sqs delete-message-batch", Err("An error occurred (ThrottlingException) when calling the DeleteMessageBatch operation: Rate exceeded".into())));
        let runner = FakeRunner::answering(answers);
        let evidence = Arc::new(FakeEvidence::default());
        let change = change(runner.clone(), evidence.clone(), true);
        let report = change.drain(None, URL, 10, 30, true).await.unwrap();
        assert_eq!(report["mode"], "stopped");
        assert_eq!(report["stoppedBy"], "failure");
        assert!(
            report["error"]
                .as_str()
                .unwrap()
                .contains("still in the queue"),
            "{report}"
        );
        assert_eq!(report["received"], 2);
        assert_eq!(report["deleted"], 1);
        assert_eq!(report["evidence"], "/evidence/demo-orders-dlq.jsonl");
        assert_eq!(evidence.lines.lock().unwrap().len(), 3);

        runner.set({
            let mut a = dlq();
            a.push((
                "sqs receive-message",
                Err("Unable to locate credentials".into()),
            ));
            a
        });
        let report = change.drain(None, URL, 10, 30, true).await.unwrap();
        assert_eq!(report["mode"], "stopped");
        assert_eq!(report["deleted"], 0);
        assert!(
            report["error"]
                .as_str()
                .unwrap()
                .contains("no AWS credentials")
        );
    }

    #[tokio::test]
    async fn every_refusal_comes_before_any_write() {
        let runner = FakeRunner::answering(dlq());
        let off = change(runner.clone(), Arc::new(FakeEvidence::default()), false);
        for result in [
            off.send(None, URL, &json!([{"body": "x"}]), None, true)
                .await,
            off.publish(None, TOPIC, &json!([{"body": "x"}]), None, true)
                .await,
            off.redrive(None, URL, None, None, true).await,
            off.redrive_cancel(None, URL, true).await,
            off.drain(None, URL, 1, 30, true).await,
        ] {
            assert!(result.unwrap_err().to_string().contains("MESSAGING_WRITES"));
        }
        assert!(runner.verbs().is_empty());
        let on = change(
            runner.clone(),
            Arc::new(FakeEvidence {
                fail: true,
                ..FakeEvidence::default()
            }),
            true,
        );
        assert!(on.drain(None, URL, 0, 30, true).await.is_err());
        assert!(on.drain(None, URL, 1001, 30, true).await.is_err());
        assert!(on.drain(None, URL, 1, -5, true).await.is_err());
        assert!(on.drain(None, "bad name", 1, 30, true).await.is_err());
        assert!(on.redrive(None, URL, None, Some(0), false).await.is_err());
        assert!(on.send(None, URL, &json!([]), None, false).await.is_err());
        let error = on
            .drain(None, URL, 5, 30, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("evidence dir not writable"), "{error}");
        assert!(runner.writes().is_empty());
    }
}
