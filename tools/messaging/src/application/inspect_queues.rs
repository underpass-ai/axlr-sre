use std::sync::Arc;

use anyhow::{Result, bail};
use aws_mcp::application::shared::{next_token, run_json};
use aws_mcp::domain::{Invocation, MetricQuery, Regions, TimeWindow, series_of};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::QueueLookup;
use crate::domain::{
    Call, InputError, MessageSummary, PEEK_MAX, QUEUES_MAX, RECEIVE_MAX, TOPICS_MAX, move_tasks,
    queue_name_of, received_messages, subscriptions, topic_facts, visibility,
};

const DEPTH_QUEUES_MAX: usize = 50;
const HISTORY_SECONDS: i64 = 24 * 3600;
const HISTORY_PERIOD: i64 = 300;
const PAGES_MAX: usize = 20;

/// The reads: `queues`, `queue`, `peek`, `redrive_status`, `topics`, `topic`.
pub struct InspectQueues {
    lookup: Arc<QueueLookup>,
    regions: Regions,
}

impl InspectQueues {
    pub fn new(lookup: Arc<QueueLookup>, regions: Regions) -> Self {
        Self { lookup, regions }
    }

    /// The queues of a region by name prefix; with `with_depth`, the depth
    /// of the first fifty.
    pub async fn queues(
        &self,
        region: Option<&str>,
        prefix: Option<&str>,
        limit: i64,
        with_depth: bool,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let mut urls: Vec<String> = Vec::new();
        let mut token: Option<String> = None;
        let mut more = false;
        let wanted = usize::try_from(limit).unwrap_or(usize::MAX);
        for _ in 0..PAGES_MAX {
            let answer = run_json(
                self.lookup.runner().as_ref(),
                &Call::list_queues(&region, prefix, limit, token.as_deref())?,
                self.lookup.profile(),
            )
            .await?;
            urls.extend(
                answer["QueueUrls"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|u| u.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            );
            token = next_token(&answer);
            if urls.len() >= wanted {
                more = token.is_some() || urls.len() > wanted;
                urls.truncate(wanted);
                break;
            }
            if token.is_none() {
                break;
            }
        }
        if token.is_some() {
            more = true;
        }
        let mut queues = Vec::new();
        for (index, url) in urls.iter().enumerate() {
            let mut row =
                json!({"name": queue_name_of(url), "url": url, "fifo": url.ends_with(".fifo")});
            if with_depth && index < DEPTH_QUEUES_MAX {
                // Attributes only, one spawn per queue; whether it is a
                // dead-letter queue is `queue`'s question.
                match self.lookup.attributes(&region, url).await {
                    Ok(facts) => {
                        row["depth"] = facts.depth.to_json();
                        row["hasRedrivePolicy"] = json!(facts.redrive.is_some());
                    }
                    Err(error) => row["error"] = json!(format!("{error:#}")),
                }
            }
            queues.push(row);
        }
        Ok(json!({
            "region": region,
            "prefix": prefix,
            "count": queues.len(),
            "more": more,
            "depthShownFor": if with_depth { queues.len().min(DEPTH_QUEUES_MAX) } else { 0 },
            "queues": queues,
        }))
    }

    /// One queue: its attributes, its sources when it is a dead-letter
    /// queue and, with `history`, the visible messages over the last day
    /// and the age of the oldest from CloudWatch.
    pub async fn queue(&self, region: Option<&str>, queue: &str, history: bool) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let facts = self.lookup.resolve(&region, queue).await?;
        let mut answer = facts.to_json();
        answer["region"] = json!(region);
        if history {
            let dimensions = [("QueueName".to_string(), facts.name.clone())];
            let queries = vec![
                MetricQuery::new(
                    "visible",
                    "AWS/SQS",
                    "ApproximateNumberOfMessagesVisible",
                    &dimensions,
                    "Maximum",
                    HISTORY_PERIOD,
                )?,
                MetricQuery::new(
                    "oldest_age",
                    "AWS/SQS",
                    "ApproximateAgeOfOldestMessage",
                    &dimensions,
                    "Maximum",
                    HISTORY_PERIOD,
                )?,
            ];
            let window = TimeWindow::parse(None, None, Timestamp::now(), HISTORY_SECONDS)?;
            match run_json(
                self.lookup.runner().as_ref(),
                &Invocation::metric_data(&region, &queries, &window)?,
                self.lookup.profile(),
            )
            .await
            {
                Ok(data) => {
                    answer["history"] = json!({
                        "start": window.start_iso(),
                        "end": window.end_iso(),
                        "visible": series_of(&data, "visible").compact(false),
                        "oldestAgeSeconds": series_of(&data, "oldest_age").last_value(),
                    });
                }
                Err(error) => answer["history"] = json!({"error": format!("{error:#}")}),
            }
        }
        Ok(answer)
    }

    /// A look at the messages of a queue without deleting them. The
    /// messages stay hidden for `visibility_seconds`; a queue with a
    /// redrive policy is refused, since a receive counts as a delivery
    /// attempt and can move its messages to the dead-letter queue.
    pub async fn peek(
        &self,
        region: Option<&str>,
        queue: &str,
        limit: i64,
        visibility_seconds: i64,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        if !(1..=PEEK_MAX).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: PEEK_MAX
            });
        }
        let visibility_seconds = visibility(visibility_seconds)?;
        let facts = self.lookup.resolve(&region, queue).await?;
        if facts.redrive.is_some() {
            bail!(InputError::SourceQueuePeek(facts.name));
        }
        let mut messages = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut empty_receives = 0;
        while (messages.len() as i64) < limit && empty_receives < 3 {
            let batch = (limit - messages.len() as i64).min(RECEIVE_MAX);
            let answer = run_json(
                self.lookup.runner().as_ref(),
                &Call::receive(&region, &facts.url, batch, visibility_seconds)?,
                self.lookup.profile(),
            )
            .await?;
            // With a short visibility the same messages come back: a batch
            // that brings nothing new counts as empty.
            let fresh: Vec<_> = received_messages(&answer)
                .into_iter()
                .filter(|m| seen.insert(m.id.clone()))
                .collect();
            if fresh.is_empty() {
                empty_receives += 1;
            } else {
                messages.extend(fresh);
            }
        }
        let summary = MessageSummary::of(&messages);
        Ok(json!({
            "region": region,
            "queue": facts.name,
            "depth": facts.depth.to_json(),
            "hiddenForSeconds": visibility_seconds,
            "summary": summary.to_json(),
            "messages": messages.iter().map(|m| m.to_json()).collect::<Vec<_>>(),
        }))
    }

    /// The message move tasks of a queue: running, completed, failed.
    pub async fn redrive_status(&self, region: Option<&str>, queue: &str) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let facts = self.lookup.resolve(&region, queue).await?;
        let answer = run_json(
            self.lookup.runner().as_ref(),
            &Call::list_move_tasks(&region, &facts.arn)?,
            self.lookup.profile(),
        )
        .await?;
        let tasks = move_tasks(&answer);
        Ok(json!({
            "region": region,
            "queue": facts.name,
            "depth": facts.depth.to_json(),
            "running": tasks.iter().any(|t| t.is_running()),
            "tasks": tasks.iter().map(|t| t.to_json()).collect::<Vec<_>>(),
        }))
    }

    pub async fn topics(&self, region: Option<&str>, limit: i64) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        if !(1..=TOPICS_MAX).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: TOPICS_MAX
            });
        }
        let answer = run_json(
            self.lookup.runner().as_ref(),
            &Call::list_topics(&region, limit, None)?,
            self.lookup.profile(),
        )
        .await?;
        let topics: Vec<Value> = answer["Topics"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|t| t["TopicArn"].as_str())
                    .map(|arn| json!({"arn": arn, "name": arn.rsplit(':').next().unwrap_or(arn), "fifo": arn.ends_with(".fifo")}))
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "region": region,
            "count": topics.len(),
            "more": next_token(&answer).is_some(),
            "topics": topics,
        }))
    }

    pub async fn topic(&self, region: Option<&str>, topic_arn: &str) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let attributes = run_json(
            self.lookup.runner().as_ref(),
            &Call::topic_attributes(&region, topic_arn)?,
            self.lookup.profile(),
        )
        .await?;
        let subs = run_json(
            self.lookup.runner().as_ref(),
            &Call::subscriptions(&region, topic_arn)?,
            self.lookup.profile(),
        )
        .await?;
        let mut answer = topic_facts(topic_arn.trim(), &attributes);
        answer["region"] = json!(region);
        answer["subscriptions"] = json!(subscriptions(&subs));
        answer["subscriptionsMore"] = json!(next_token(&subs).is_some());
        Ok(answer)
    }

    /// For a sibling use case: the queues limit, so the catalogue and the
    /// use case agree.
    #[must_use]
    pub const fn queues_max() -> i64 {
        QUEUES_MAX
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{
        ARN, FakeRunner, SOURCE, TOPIC, URL, dlq_attributes, message,
    };
    use super::*;

    fn inspect(runner: &Arc<FakeRunner>) -> InspectQueues {
        let runner: Arc<dyn aws_mcp::ports::AwsRunner> = runner.clone();
        InspectQueues::new(
            Arc::new(QueueLookup::new(runner, None)),
            Regions::new(Some("eu-west-1")).unwrap(),
        )
    }

    #[tokio::test]
    async fn queues_are_listed_with_optional_depth() {
        let runner = FakeRunner::answering(vec![
            ("--starting-token t", Ok(json!({"QueueUrls": [SOURCE]}))),
            (
                "sqs list-queues",
                Ok(json!({"QueueUrls": [URL], "NextToken": "t"})),
            ),
            ("sqs get-queue-attributes", Ok(dlq_attributes(2))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
        ]);
        let answer = inspect(&runner)
            .queues(None, Some("demo"), 10, true)
            .await
            .unwrap();
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["more"], false);
        assert_eq!(answer["queues"][0]["name"], "demo-orders-dlq");
        assert_eq!(answer["queues"][0]["depth"]["visible"], 2);
        assert_eq!(answer["queues"][0]["hasRedrivePolicy"], false);
        assert!(answer["queues"][0].get("isDeadLetter").is_none());
        assert_eq!(answer["depthShownFor"], 2);
        assert!(
            !runner
                .verbs()
                .iter()
                .any(|v| v == "sqs list-dead-letter-source-queues"),
            "attributes only"
        );
        let one = inspect(&runner).queues(None, None, 1, false).await.unwrap();
        assert_eq!(one["count"], 1);
        assert_eq!(one["more"], true);
        assert!(one["queues"][0].get("depth").is_none());
        assert!(inspect(&runner).queues(None, None, 0, false).await.is_err());
        runner.set(vec![(
            "sqs list-queues",
            Ok(json!({"QueueUrls": [URL], "NextToken": "loop"})),
        )]);
        let looping = inspect(&runner)
            .queues(None, None, 100, false)
            .await
            .unwrap();
        assert_eq!(looping["count"], PAGES_MAX);
        assert_eq!(looping["more"], true);
        runner.set(vec![
            ("sqs list-queues", Ok(json!({"QueueUrls": [URL]}))),
            ("sqs get-queue-attributes", Err("An error occurred (AccessDeniedException) when calling the GetQueueAttributes operation: no".into())),
        ]);
        let denied = inspect(&runner).queues(None, None, 10, true).await.unwrap();
        assert!(
            denied["queues"][0]["error"]
                .as_str()
                .unwrap()
                .contains("AccessDenied")
        );
        assert_eq!(InspectQueues::queues_max(), 100);
    }

    #[tokio::test]
    async fn one_queue_with_its_history() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(2))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "cloudwatch get-metric-data",
                Ok(
                    json!({"MetricDataResults": [{"Id": "visible", "Timestamps": ["2026-11-03T09:00:00+00:00"], "Values": [2.0]}, {"Id": "oldest_age", "Timestamps": ["2026-11-03T09:00:00+00:00"], "Values": [600.0]}]}),
                ),
            ),
        ]);
        let answer = inspect(&runner).queue(None, URL, true).await.unwrap();
        assert_eq!(answer["name"], "demo-orders-dlq");
        assert_eq!(answer["sources"][0], SOURCE);
        assert_eq!(answer["history"]["oldestAgeSeconds"], 600.0);
        assert_eq!(answer["history"]["visible"][0][1], 2.0);
        assert!(
            runner.lines()[2].contains("QueueName")
                && runner.lines()[2].contains("demo-orders-dlq")
        );
        runner.set(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(2))),
            ("sqs list-dead-letter-source-queues", Ok(json!({"queueUrls": []}))),
            ("cloudwatch get-metric-data", Err("An error occurred (AccessDeniedException) when calling the GetMetricData operation: no".into())),
        ]);
        let partial = inspect(&runner).queue(None, URL, true).await.unwrap();
        assert!(
            partial["history"]["error"]
                .as_str()
                .unwrap()
                .contains("AccessDenied")
        );
        let plain = inspect(&runner).queue(None, URL, false).await.unwrap();
        assert!(plain.get("history").is_none());
        assert!(
            inspect(&runner)
                .queue(Some("mars"), URL, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn peek_receives_in_batches_and_refuses_a_source_queue() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(12))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "once:sqs receive-message",
                Ok(
                    json!({"Messages": (0..10).map(|i| message(&format!("m{i}"), "same")).collect::<Vec<_>>()}),
                ),
            ),
            (
                "once:sqs receive-message",
                Ok(json!({"Messages": [message("m10", "other")]})),
            ),
            ("sqs receive-message", Ok(Value::Null)),
        ]);
        let answer = inspect(&runner).peek(None, URL, 15, 20).await.unwrap();
        assert_eq!(answer["summary"]["count"], 11);
        assert_eq!(answer["summary"]["uniqueBodies"], 2);
        assert_eq!(answer["summary"]["receiveCounts"]["5"], 11);
        assert_eq!(answer["hiddenForSeconds"], 20);
        assert_eq!(answer["messages"][0]["id"], "m0");
        let receives = runner
            .verbs()
            .iter()
            .filter(|v| *v == "sqs receive-message")
            .count();
        assert_eq!(receives, 5, "two with messages, three empty");
        assert!(runner.lines()[2].contains("--max-number-of-messages 10"));
        assert!(runner.lines()[3].contains("--max-number-of-messages 5"));
        assert!(runner.writes().is_empty());

        let mut source = dlq_attributes(1);
        source["Attributes"]["RedrivePolicy"] =
            json!("{\"deadLetterTargetArn\":\"a\",\"maxReceiveCount\":3}");
        runner.set(vec![
            ("sqs get-queue-attributes", Ok(source)),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": []})),
            ),
        ]);
        let error = inspect(&runner)
            .peek(None, URL, 5, 30)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("delivery attempt"), "{error}");
        assert!(inspect(&runner).peek(None, URL, 0, 30).await.is_err());
        assert!(inspect(&runner).peek(None, URL, 101, 30).await.is_err());
        assert!(inspect(&runner).peek(None, URL, 5, 43_201).await.is_err());
    }

    #[tokio::test]
    async fn peek_with_a_short_visibility_does_not_repeat_messages() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(2))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "sqs receive-message",
                Ok(json!({"Messages": [message("m1", "a"), message("m2", "b")]})),
            ),
        ]);
        let answer = inspect(&runner).peek(None, URL, 10, 0).await.unwrap();
        assert_eq!(answer["summary"]["count"], 2);
        let receives = runner
            .verbs()
            .iter()
            .filter(|v| *v == "sqs receive-message")
            .count();
        assert_eq!(
            receives, 4,
            "one with new ids, three that brought nothing new"
        );
    }

    #[tokio::test]
    async fn redrive_status_topics_and_a_topic() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-attributes", Ok(dlq_attributes(2))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "sqs list-message-move-tasks",
                Ok(json!({"Results": [{"TaskHandle": "h", "Status": "RUNNING"}]})),
            ),
            (
                "sns list-topics",
                Ok(json!({"Topics": [{"TopicArn": TOPIC}], "NextToken": "t"})),
            ),
            (
                "sns get-topic-attributes",
                Ok(json!({"Attributes": {"DisplayName": "Demo", "SubscriptionsConfirmed": "1"}})),
            ),
            (
                "sns list-subscriptions-by-topic",
                Ok(
                    json!({"Subscriptions": [{"SubscriptionArn": "s", "Protocol": "sqs", "Endpoint": ARN}]}),
                ),
            ),
        ]);
        let inspect = inspect(&runner);
        let status = inspect.redrive_status(None, "demo-orders-dlq").await;
        assert!(status.is_err(), "a name needs get-queue-url");
        let status = inspect.redrive_status(None, URL).await.unwrap();
        assert_eq!(status["running"], true);
        assert_eq!(status["tasks"][0]["handle"], "h");
        assert!(
            runner
                .lines()
                .iter()
                .any(|l| l.contains(&format!("--source-arn {ARN}")))
        );
        let topics = inspect.topics(None, 10).await.unwrap();
        assert_eq!(topics["count"], 1);
        assert_eq!(topics["more"], true);
        assert_eq!(topics["topics"][0]["name"], "demo-topic");
        assert!(inspect.topics(None, 0).await.is_err());
        let topic = inspect.topic(None, TOPIC).await.unwrap();
        assert_eq!(topic["displayName"], "Demo");
        assert_eq!(topic["subscriptions"][0]["endpoint"], ARN);
        assert_eq!(topic["subscriptionsMore"], false);
        assert!(
            inspect
                .topic(None, "arn:aws:sns:us-east-1:123456789012:t")
                .await
                .is_err()
        );
    }
}
