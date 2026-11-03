use aws_mcp::domain::Invocation;
use serde_json::Value;

use super::InputError;

pub const QUEUES_MAX: i64 = 100;
pub const TOPICS_MAX: i64 = 100;
pub const RECEIVE_MAX: i64 = 10;
pub const PEEK_MAX: i64 = 100;
pub const MESSAGES_MAX: usize = 100;
pub const BATCH_SIZE: usize = 10;
pub const REDRIVE_RATE_MAX: i64 = 500;
const VISIBILITY_MAX: i64 = 43_200;
const WAIT_SECONDS: i64 = 1;

/// The `aws sqs` and `aws sns` runs, built from validated parts. Every
/// value is its own argument; a batch travels as one JSON argument, since
/// receipt handles carry `+`, `/` and `=`.
pub struct Call;

fn limit(what: &'static str, value: i64, max: i64) -> Result<i64, InputError> {
    if (1..=max).contains(&value) {
        Ok(value)
    } else {
        Err(InputError::OutOfRange { what, min: 1, max })
    }
}

/// How long a received message stays hidden: 0 to 43 200 seconds.
pub fn visibility(seconds: i64) -> Result<i64, InputError> {
    if (0..=VISIBILITY_MAX).contains(&seconds) {
        Ok(seconds)
    } else {
        Err(InputError::OutOfRange {
            what: "visibility_seconds",
            min: 0,
            max: VISIBILITY_MAX,
        })
    }
}

fn raw(region: &str, args: Vec<String>) -> Result<Invocation, InputError> {
    Ok(Invocation::raw(region, args)?)
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// `arn:aws:<service>:<region>:<12 digits>:<name>` in the call's region.
pub fn arn(
    what: &'static str,
    service: &str,
    value: &str,
    region: &str,
) -> Result<String, InputError> {
    let value = value.trim();
    let parts: Vec<&str> = value.split(':').collect();
    let ok = parts.len() == 6
        && parts[0] == "arn"
        && parts[1] == "aws"
        && parts[2] == service
        && parts[4].len() == 12
        && parts[4].chars().all(|c| c.is_ascii_digit())
        && !parts[5].is_empty()
        && parts[5]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        return Err(InputError::Invalid {
            what,
            value: value.chars().take(100).collect(),
            rule: "an ARN such as arn:aws:sqs:eu-west-1:123456789012:orders",
        });
    }
    if parts[3] != region {
        return Err(InputError::RegionMismatch {
            url_region: parts[3].to_string(),
            region: region.to_string(),
        });
    }
    Ok(value.to_string())
}

fn prefix(value: Option<&str>) -> Result<Option<String>, InputError> {
    match value.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p)
            if p.len() <= 80
                && !p.starts_with('-')
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) =>
        {
            Ok(Some(p.to_string()))
        }
        Some(p) => Err(InputError::Invalid {
            what: "prefix",
            value: p.chars().take(60).collect(),
            rule: "letters, digits, `-`, `_` and `.`",
        }),
    }
}

impl Call {
    pub fn list_queues(
        region: &str,
        name_prefix: Option<&str>,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Invocation, InputError> {
        let mut a = args(&["sqs", "list-queues"]);
        if let Some(prefix) = prefix(name_prefix)? {
            a.extend(args(&["--queue-name-prefix", &prefix]));
        }
        Ok(raw(region, a)?.paged(limit("limit", max_items, QUEUES_MAX)?, token))
    }

    pub fn queue_url(region: &str, name: &str) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&["sqs", "get-queue-url", "--queue-name", name]),
        )
    }

    pub fn attributes(region: &str, url: &str) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sqs",
                "get-queue-attributes",
                "--queue-url",
                url,
                "--attribute-names",
                "All",
            ]),
        )
    }

    pub fn dead_letter_sources(region: &str, url: &str) -> Result<Invocation, InputError> {
        Ok(raw(
            region,
            args(&["sqs", "list-dead-letter-source-queues", "--queue-url", url]),
        )?
        .paged(50, None))
    }

    pub fn receive(
        region: &str,
        url: &str,
        max: i64,
        visibility_seconds: i64,
    ) -> Result<Invocation, InputError> {
        let max = limit("batch", max, RECEIVE_MAX)?;
        let visibility_seconds = visibility(visibility_seconds)?;
        raw(
            region,
            args(&[
                "sqs",
                "receive-message",
                "--queue-url",
                url,
                "--max-number-of-messages",
                &max.to_string(),
                "--visibility-timeout",
                &visibility_seconds.to_string(),
                "--wait-time-seconds",
                &WAIT_SECONDS.to_string(),
                "--attribute-names",
                "All",
                "--message-attribute-names",
                "All",
            ]),
        )
    }

    pub fn delete_batch(
        region: &str,
        url: &str,
        entries: &Value,
    ) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sqs",
                "delete-message-batch",
                "--queue-url",
                url,
                "--entries",
                &entries.to_string(),
            ]),
        )
    }

    pub fn send_batch(region: &str, url: &str, entries: &Value) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sqs",
                "send-message-batch",
                "--queue-url",
                url,
                "--entries",
                &entries.to_string(),
            ]),
        )
    }

    /// Without a destination every message goes back to the queue it came from.
    pub fn start_move(
        region: &str,
        source_arn: &str,
        destination_arn: Option<&str>,
        per_second: Option<i64>,
    ) -> Result<Invocation, InputError> {
        let mut a = args(&[
            "sqs",
            "start-message-move-task",
            "--source-arn",
            &arn("source", "sqs", source_arn, region)?,
        ]);
        if let Some(destination) = destination_arn {
            a.extend(args(&[
                "--destination-arn",
                &arn("destination", "sqs", destination, region)?,
            ]));
        }
        if let Some(rate) = per_second {
            a.extend(args(&[
                "--max-number-of-messages-per-second",
                &limit("per_second", rate, REDRIVE_RATE_MAX)?.to_string(),
            ]));
        }
        raw(region, a)
    }

    pub fn list_move_tasks(region: &str, source_arn: &str) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sqs",
                "list-message-move-tasks",
                "--source-arn",
                &arn("source", "sqs", source_arn, region)?,
                "--max-results",
                "10",
            ]),
        )
    }

    pub fn cancel_move(region: &str, handle: &str) -> Result<Invocation, InputError> {
        let handle = handle.trim();
        if handle.is_empty()
            || handle.len() > 2_048
            || handle.starts_with('-')
            || handle.chars().any(char::is_control)
        {
            return Err(InputError::Invalid {
                what: "task",
                value: handle.chars().take(60).collect(),
                rule: "a task handle as redrive_status lists them",
            });
        }
        raw(
            region,
            args(&["sqs", "cancel-message-move-task", "--task-handle", handle]),
        )
    }

    pub fn list_topics(
        region: &str,
        max_items: i64,
        token: Option<&str>,
    ) -> Result<Invocation, InputError> {
        Ok(raw(region, args(&["sns", "list-topics"]))?
            .paged(limit("limit", max_items, TOPICS_MAX)?, token))
    }

    pub fn topic_attributes(region: &str, topic_arn: &str) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sns",
                "get-topic-attributes",
                "--topic-arn",
                &arn("topic", "sns", topic_arn, region)?,
            ]),
        )
    }

    pub fn subscriptions(region: &str, topic_arn: &str) -> Result<Invocation, InputError> {
        Ok(raw(
            region,
            args(&[
                "sns",
                "list-subscriptions-by-topic",
                "--topic-arn",
                &arn("topic", "sns", topic_arn, region)?,
            ]),
        )?
        .paged(50, None))
    }

    pub fn publish_batch(
        region: &str,
        topic_arn: &str,
        entries: &Value,
    ) -> Result<Invocation, InputError> {
        raw(
            region,
            args(&[
                "sns",
                "publish-batch",
                "--topic-arn",
                &arn("topic", "sns", topic_arn, region)?,
                "--publish-batch-request-entries",
                &entries.to_string(),
            ]),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const URL: &str = "https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders-dlq";
    const ARN: &str = "arn:aws:sqs:eu-west-1:123456789012:demo-orders-dlq";

    fn has(invocation: &Invocation, flag: &str, value: &str) -> bool {
        invocation
            .args()
            .windows(2)
            .any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn queue_reads_carry_their_flags() {
        let list = Call::list_queues("eu-west-1", Some("demo-"), 20, Some("t")).unwrap();
        assert_eq!(list.verb(), "sqs list-queues");
        assert!(
            has(&list, "--queue-name-prefix", "demo-")
                && has(&list, "--max-items", "20")
                && has(&list, "--starting-token", "t")
        );
        assert!(Call::list_queues("eu-west-1", Some("a b"), 20, None).is_err());
        assert!(Call::list_queues("eu-west-1", None, 0, None).is_err());
        assert!(Call::list_queues("mars", None, 1, None).is_err());
        assert!(has(
            &Call::queue_url("eu-west-1", "demo").unwrap(),
            "--queue-name",
            "demo"
        ));
        assert!(has(
            &Call::attributes("eu-west-1", URL).unwrap(),
            "--attribute-names",
            "All"
        ));
        assert_eq!(
            Call::dead_letter_sources("eu-west-1", URL).unwrap().verb(),
            "sqs list-dead-letter-source-queues"
        );
        let receive = Call::receive("eu-west-1", URL, 10, 30).unwrap();
        assert!(
            has(&receive, "--max-number-of-messages", "10")
                && has(&receive, "--visibility-timeout", "30")
                && has(&receive, "--wait-time-seconds", "1")
        );
        assert!(Call::receive("eu-west-1", URL, 11, 30).is_err());
        assert!(Call::receive("eu-west-1", URL, 1, -1).is_err());
        assert!(Call::receive("eu-west-1", URL, 1, 43_201).is_err());
    }

    #[test]
    fn writes_carry_a_json_argument_and_checked_arns() {
        let entries = json!([{"Id": "0", "ReceiptHandle": "a+b/c="}]);
        let delete = Call::delete_batch("eu-west-1", URL, &entries).unwrap();
        assert!(has(&delete, "--entries", &entries.to_string()));
        assert_eq!(
            Call::send_batch("eu-west-1", URL, &entries).unwrap().verb(),
            "sqs send-message-batch"
        );
        let start = Call::start_move(
            "eu-west-1",
            ARN,
            Some("arn:aws:sqs:eu-west-1:123456789012:demo-orders"),
            Some(50),
        )
        .unwrap();
        assert!(
            has(&start, "--source-arn", ARN)
                && has(&start, "--max-number-of-messages-per-second", "50")
        );
        let plain = Call::start_move("eu-west-1", ARN, None, None).unwrap();
        assert!(!plain.args().contains(&"--destination-arn".to_string()));
        assert!(Call::start_move("eu-west-1", ARN, None, Some(501)).is_err());
        assert!(Call::start_move("us-east-1", ARN, None, None).is_err());
        assert!(
            Call::start_move(
                "eu-west-1",
                "arn:aws:sns:eu-west-1:123456789012:t",
                None,
                None
            )
            .is_err()
        );
        assert!(Call::start_move("eu-west-1", "arn:aws:sqs:eu-west-1:12:q", None, None).is_err());
        assert_eq!(
            Call::list_move_tasks("eu-west-1", ARN).unwrap().verb(),
            "sqs list-message-move-tasks"
        );
        assert!(has(
            &Call::cancel_move("eu-west-1", "AQEB...handle").unwrap(),
            "--task-handle",
            "AQEB...handle"
        ));
        for bad in ["", "-x", "a\nb"] {
            assert!(Call::cancel_move("eu-west-1", bad).is_err(), "{bad}");
        }
        let topic = "arn:aws:sns:eu-west-1:123456789012:demo-topic.fifo";
        assert!(has(
            &Call::topic_attributes("eu-west-1", topic).unwrap(),
            "--topic-arn",
            topic
        ));
        assert!(has(
            &Call::list_topics("eu-west-1", 5, None).unwrap(),
            "--max-items",
            "5"
        ));
        assert!(Call::list_topics("eu-west-1", 101, None).is_err());
        assert_eq!(
            Call::subscriptions("eu-west-1", topic).unwrap().verb(),
            "sns list-subscriptions-by-topic"
        );
        assert!(has(
            &Call::publish_batch("eu-west-1", topic, &json!([])).unwrap(),
            "--publish-batch-request-entries",
            "[]"
        ));
        assert!(Call::publish_batch("eu-west-1", ARN, &json!([])).is_err());
    }
}
