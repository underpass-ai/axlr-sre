use serde_json::{Value, json};

use super::InputError;

const NAME_MAX: usize = 80;

/// How a call names a queue: its URL, or its name (resolved with
/// `get-queue-url` in the call's region).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueRef {
    Url(String),
    Name(String),
}

impl QueueRef {
    /// A URL is `https://sqs.<region>.amazonaws.com/<12 digits>/<name>` and
    /// must be in the call's region; a name is up to 80 characters of
    /// letters, digits, `-` and `_`, with `.fifo` allowed.
    pub fn parse(raw: &str, region: &str) -> Result<Self, InputError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(InputError::Empty("queue"));
        }
        if raw.contains("://") {
            let url_region = url_region(raw).ok_or_else(|| InputError::Invalid {
                what: "queue",
                value: raw.chars().take(100).collect(),
                rule: "a queue URL such as https://sqs.eu-west-1.amazonaws.com/123456789012/orders",
            })?;
            if url_region != region {
                return Err(InputError::RegionMismatch {
                    url_region,
                    region: region.to_string(),
                });
            }
            return Ok(Self::Url(raw.to_string()));
        }
        if !valid_name(raw) {
            return Err(InputError::Invalid {
                what: "queue",
                value: raw.chars().take(100).collect(),
                rule: "a queue name: letters, digits, `-` and `_`, 80 at most, `.fifo` allowed",
            });
        }
        Ok(Self::Name(raw.to_string()))
    }
}

fn valid_name(name: &str) -> bool {
    let base = name.strip_suffix(".fifo").unwrap_or(name);
    !base.is_empty()
        && name.chars().count() <= NAME_MAX
        && !name.starts_with('-')
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn url_region(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://sqs.")?;
    let (host, path) = rest.split_once('/')?;
    let region = host.strip_suffix(".amazonaws.com")?;
    let (account, name) = path.split_once('/')?;
    let ok = aws_mcp::domain::region(region).is_ok()
        && account.len() == 12
        && account.chars().all(|c| c.is_ascii_digit())
        && valid_name(name);
    ok.then(|| region.to_string())
}

/// The last segment of a queue URL.
#[must_use]
pub fn queue_name_of(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_string()
}

/// How many messages a queue holds, from its attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Depth {
    pub visible: u64,
    pub in_flight: u64,
    pub delayed: u64,
}

impl Depth {
    #[must_use]
    pub fn to_json(self) -> Value {
        json!({"visible": self.visible, "inFlight": self.in_flight, "delayed": self.delayed})
    }
}

/// Where a queue sends what fails, and after how many receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedrivePolicy {
    pub dead_letter_arn: String,
    pub max_receive_count: u64,
}

/// What `get-queue-attributes --attribute-names All` says about a queue.
/// Every value arrives as a string; `RedrivePolicy` is a JSON string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueFacts {
    pub url: String,
    pub name: String,
    pub arn: String,
    pub fifo: bool,
    pub content_based_deduplication: bool,
    pub depth: Depth,
    pub redrive: Option<RedrivePolicy>,
    pub visibility_timeout: Option<u64>,
    pub retention_seconds: Option<u64>,
    pub created: Option<String>,
    /// The queues whose failed messages land here; filled from
    /// `list-dead-letter-source-queues`.
    pub sources: Vec<String>,
}

impl QueueFacts {
    pub fn read(url: &str, answer: &Value) -> Option<Self> {
        let attributes = answer["Attributes"].as_object()?;
        let text = |key: &str| attributes.get(key).and_then(Value::as_str);
        let number = |key: &str| text(key).and_then(|v| v.parse::<u64>().ok());
        let flag = |key: &str| text(key).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let redrive = text("RedrivePolicy")
            .and_then(|policy| serde_json::from_str::<Value>(policy).ok())
            .and_then(|policy| {
                Some(RedrivePolicy {
                    dead_letter_arn: policy["deadLetterTargetArn"].as_str()?.to_string(),
                    max_receive_count: match &policy["maxReceiveCount"] {
                        Value::Number(n) => n.as_u64().unwrap_or(0),
                        Value::String(s) => s.parse().unwrap_or(0),
                        _ => 0,
                    },
                })
            });
        Some(Self {
            url: url.to_string(),
            name: queue_name_of(url),
            arn: text("QueueArn").unwrap_or_default().to_string(),
            fifo: flag("FifoQueue") || url.ends_with(".fifo"),
            content_based_deduplication: flag("ContentBasedDeduplication"),
            depth: Depth {
                visible: number("ApproximateNumberOfMessages").unwrap_or(0),
                in_flight: number("ApproximateNumberOfMessagesNotVisible").unwrap_or(0),
                delayed: number("ApproximateNumberOfMessagesDelayed").unwrap_or(0),
            },
            redrive,
            visibility_timeout: number("VisibilityTimeout"),
            retention_seconds: number("MessageRetentionPeriod"),
            created: number("CreatedTimestamp")
                .and_then(|s| jiff::Timestamp::from_second(s as i64).ok())
                .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
            sources: Vec::new(),
        })
    }

    /// A dead-letter queue is one some queue sends its failed messages to.
    #[must_use]
    pub fn is_dead_letter(&self) -> bool {
        !self.sources.is_empty()
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "url": self.url,
            "arn": self.arn,
            "fifo": self.fifo,
            "contentBasedDeduplication": self.content_based_deduplication,
            "depth": self.depth.to_json(),
            "redrivePolicy": self.redrive.as_ref().map(|r| json!({"deadLetterArn": r.dead_letter_arn, "maxReceiveCount": r.max_receive_count})),
            "isDeadLetter": self.is_dead_letter(),
            "sources": self.sources,
            "visibilityTimeout": self.visibility_timeout,
            "retentionSeconds": self.retention_seconds,
            "created": self.created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders-dlq";

    #[test]
    fn a_queue_is_a_url_in_the_region_or_a_name() {
        assert_eq!(
            QueueRef::parse(URL, "eu-west-1").unwrap(),
            QueueRef::Url(URL.into())
        );
        assert_eq!(
            QueueRef::parse(" demo-orders.fifo ", "eu-west-1").unwrap(),
            QueueRef::Name("demo-orders.fifo".into())
        );
        assert_eq!(
            QueueRef::parse(URL, "us-east-1").unwrap_err(),
            InputError::RegionMismatch {
                url_region: "eu-west-1".into(),
                region: "us-east-1".into()
            }
        );
        assert_eq!(
            QueueRef::parse("", "eu-west-1").unwrap_err(),
            InputError::Empty("queue")
        );
        for bad in [
            "https://example.com/q",
            "https://sqs.eu-west-1.amazonaws.com/12/q",
            "https://sqs.eu-west-1.amazonaws.com/123456789012/bad name",
            "-q",
            "a b",
            ".fifo",
            &"q".repeat(81),
        ] {
            assert!(QueueRef::parse(bad, "eu-west-1").is_err(), "{bad}");
        }
        assert_eq!(queue_name_of(URL), "demo-orders-dlq");
    }

    #[test]
    fn the_facts_are_read_from_the_attributes() {
        let answer = json!({"Attributes": {
            "QueueArn": "arn:aws:sqs:eu-west-1:123456789012:demo-orders-dlq",
            "ApproximateNumberOfMessages": "12", "ApproximateNumberOfMessagesNotVisible": "1", "ApproximateNumberOfMessagesDelayed": "0",
            "VisibilityTimeout": "30", "MessageRetentionPeriod": "1209600", "CreatedTimestamp": "1793707200",
            "FifoQueue": "false",
            "RedrivePolicy": "{\"deadLetterTargetArn\":\"arn:aws:sqs:eu-west-1:123456789012:demo-dlq\",\"maxReceiveCount\":5}"
        }});
        let mut facts = QueueFacts::read(URL, &answer).unwrap();
        assert_eq!(facts.name, "demo-orders-dlq");
        assert_eq!(
            facts.depth,
            Depth {
                visible: 12,
                in_flight: 1,
                delayed: 0
            }
        );
        assert_eq!(facts.redrive.as_ref().unwrap().max_receive_count, 5);
        assert_eq!(facts.created.as_deref(), Some("2026-11-03T12:00:00Z"));
        assert!(!facts.fifo && !facts.is_dead_letter());
        facts
            .sources
            .push("https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders".into());
        assert!(facts.is_dead_letter());
        let json = facts.to_json();
        assert_eq!(json["depth"]["visible"], 12);
        assert_eq!(json["redrivePolicy"]["maxReceiveCount"], 5);
        assert_eq!(json["isDeadLetter"], true);
        let fifo = QueueFacts::read(
            "https://sqs.eu-west-1.amazonaws.com/123456789012/q.fifo",
            &json!({"Attributes": {"RedrivePolicy": "{\"deadLetterTargetArn\":\"a\",\"maxReceiveCount\":\"3\"}", "ContentBasedDeduplication": "true"}}),
        )
        .unwrap();
        assert!(fifo.fifo && fifo.content_based_deduplication);
        assert_eq!(fifo.redrive.unwrap().max_receive_count, 3);
        assert_eq!(fifo.depth, Depth::default());
        assert!(QueueFacts::read(URL, &json!({})).is_none());
        assert!(
            QueueFacts::read(URL, &json!({"Attributes": {"RedrivePolicy": "nope"}}))
                .unwrap()
                .redrive
                .is_none()
        );
    }
}
