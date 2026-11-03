use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::{BATCH_SIZE, InputError, MESSAGES_MAX};

const BODY_SHOWN_BYTES: usize = 4_096;
const BODY_MAX_BYTES: usize = 262_144;
const BATCH_MAX_BYTES: usize = 262_144;
const ATTRIBUTES_MAX: usize = 10;
const ID_MAX: usize = 128;

/// A message as `receive-message` answered it. The body is kept whole for
/// the evidence file and cut to 4 KiB in answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMessage {
    pub id: String,
    pub receipt_handle: String,
    pub body: String,
    pub sent: Option<String>,
    pub receive_count: Option<u64>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

impl ReceivedMessage {
    fn read(raw: &Value) -> Option<Self> {
        let system = raw["Attributes"].as_object();
        let text = |key: &str| {
            system
                .and_then(|a| a.get(key))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        Some(Self {
            id: raw["MessageId"].as_str()?.to_string(),
            receipt_handle: raw["ReceiptHandle"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            body: raw["Body"].as_str().unwrap_or_default().to_string(),
            sent: text("SentTimestamp")
                .and_then(|ms| ms.parse::<i64>().ok())
                .and_then(|ms| Timestamp::from_millisecond(ms).ok())
                .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
            receive_count: text("ApproximateReceiveCount").and_then(|c| c.parse().ok()),
            group_id: text("MessageGroupId"),
            deduplication_id: text("MessageDeduplicationId"),
            attributes: raw["MessageAttributes"]
                .as_object()
                .map(|attributes| {
                    attributes
                        .iter()
                        .map(|(name, value)| {
                            let shown = value["StringValue"]
                                .as_str()
                                .map(|s| s.chars().take(200).collect())
                                .unwrap_or_else(|| {
                                    format!("<{}>", value["DataType"].as_str().unwrap_or("binary"))
                                });
                            (name.clone(), shown)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// The row an answer shows: the body cut, no receipt handle.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let (body, cut) = cut_body(&self.body);
        json!({
            "id": self.id,
            "sent": self.sent,
            "receiveCount": self.receive_count,
            "groupId": self.group_id,
            "deduplicationId": self.deduplication_id,
            "body": body,
            "bodyCut": cut,
            "attributes": self.attributes,
        })
    }

    /// The line the evidence file keeps: everything, body whole.
    #[must_use]
    pub fn evidence_line(&self) -> Value {
        json!({
            "id": self.id,
            "sent": self.sent,
            "receiveCount": self.receive_count,
            "groupId": self.group_id,
            "deduplicationId": self.deduplication_id,
            "body": self.body,
            "attributes": self.attributes,
        })
    }
}

fn cut_body(body: &str) -> (String, bool) {
    if body.len() <= BODY_SHOWN_BYTES {
        return (body.to_string(), false);
    }
    let mut end = BODY_SHOWN_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    (body[..end].to_string(), true)
}

/// The messages of a `receive-message` answer; an empty receive is an
/// empty answer, which the CLI prints as nothing.
#[must_use]
pub fn received_messages(answer: &Value) -> Vec<ReceivedMessage> {
    answer["Messages"]
        .as_array()
        .map(|items| items.iter().filter_map(ReceivedMessage::read).collect())
        .unwrap_or_default()
}

/// What to read before deciding what to do with the messages: whether they
/// are one message repeated, one FIFO group, and how many times each has
/// been tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSummary {
    pub count: usize,
    pub unique_bodies: usize,
    pub groups: BTreeMap<String, usize>,
    pub receive_counts: BTreeMap<String, usize>,
    pub oldest_sent: Option<String>,
}

impl MessageSummary {
    #[must_use]
    pub fn of(messages: &[ReceivedMessage]) -> Self {
        let mut bodies = std::collections::BTreeSet::new();
        let mut groups = BTreeMap::new();
        let mut receive_counts = BTreeMap::new();
        let mut oldest: Option<String> = None;
        for message in messages {
            bodies.insert(message.body.as_str());
            *groups
                .entry(message.group_id.clone().unwrap_or_else(|| "-".into()))
                .or_insert(0) += 1;
            *receive_counts
                .entry(
                    message
                        .receive_count
                        .map_or("-".to_string(), |c| c.to_string()),
                )
                .or_insert(0) += 1;
            if let Some(sent) = &message.sent
                && oldest.as_ref().is_none_or(|o| sent < o)
            {
                oldest = Some(sent.clone());
            }
        }
        Self {
            count: messages.len(),
            unique_bodies: bodies.len(),
            groups,
            receive_counts,
            oldest_sent: oldest,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "count": self.count,
            "uniqueBodies": self.unique_bodies,
            "groups": self.groups,
            "receiveCounts": self.receive_counts,
            "oldestSent": self.oldest_sent,
        })
    }
}

/// One message to send or publish, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMessage {
    pub body: String,
    pub attributes: BTreeMap<String, String>,
    pub group_id: Option<String>,
    pub deduplication_id: Option<String>,
    pub subject: Option<String>,
}

/// One to a hundred messages for one destination, in batches of ten the
/// APIs take, each batch under 256 KiB. A FIFO destination needs a group
/// id on every message or a default, and a deduplication id unless the
/// destination deduplicates by content, in which case one is derived from
/// the body; a standard destination refuses the FIFO fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMessages {
    messages: Vec<OutboundMessage>,
}

impl OutboundMessages {
    pub fn parse(
        raw: &Value,
        default_group_id: Option<&str>,
        fifo: bool,
        content_based_deduplication: bool,
    ) -> Result<Self, InputError> {
        let items = raw.as_array().ok_or(InputError::Empty("messages"))?;
        if items.is_empty() || items.len() > MESSAGES_MAX {
            return Err(InputError::OutOfRange {
                what: "messages",
                min: 1,
                max: MESSAGES_MAX as i64,
            });
        }
        let default_group_id = match default_group_id.map(str::trim).filter(|g| !g.is_empty()) {
            Some(id) => Some(identifier("message_group_id", id)?),
            None => None,
        };
        let mut messages = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let body = match item.get("body") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Null) | None => return Err(InputError::Empty("message body")),
                Some(other) => other.to_string(),
            };
            if body.is_empty() || body.len() > BODY_MAX_BYTES {
                return Err(InputError::OutOfRange {
                    what: "message body bytes",
                    min: 1,
                    max: BODY_MAX_BYTES as i64,
                });
            }
            let attributes = attributes_of(item.get("attributes"))?;
            let given_group = item["message_group_id"]
                .as_str()
                .map(str::trim)
                .filter(|g| !g.is_empty());
            let given_dedup = item["message_deduplication_id"]
                .as_str()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            let (group_id, deduplication_id) = if fifo {
                let group = match given_group.or(default_group_id.as_deref()) {
                    Some(group) => identifier("message_group_id", group)?,
                    None => {
                        return Err(InputError::FifoNeeds {
                            what: "messages",
                            field: "message_group_id",
                        });
                    }
                };
                let dedup = match given_dedup {
                    Some(id) => identifier("message_deduplication_id", id)?,
                    None if content_based_deduplication => derived_id(&body, index),
                    None => {
                        return Err(InputError::FifoNeeds {
                            what: "messages",
                            field: "message_deduplication_id",
                        });
                    }
                };
                (Some(group), Some(dedup))
            } else {
                if given_group.is_some() || given_dedup.is_some() || default_group_id.is_some() {
                    return Err(InputError::NotFifo("messages"));
                }
                (None, None)
            };
            let subject = item["subject"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(100).collect());
            messages.push(OutboundMessage {
                body,
                attributes,
                group_id,
                deduplication_id,
                subject,
            });
        }
        Ok(Self { messages })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Batches of ten at most and 256 KiB at most, with each message's index.
    #[must_use]
    pub fn batches(&self) -> Vec<Vec<(usize, &OutboundMessage)>> {
        let mut batches: Vec<Vec<(usize, &OutboundMessage)>> = Vec::new();
        let mut current: Vec<(usize, &OutboundMessage)> = Vec::new();
        let mut bytes = 0usize;
        for (index, message) in self.messages.iter().enumerate() {
            let size = message.body.len();
            if !current.is_empty()
                && (current.len() == BATCH_SIZE || bytes + size > BATCH_MAX_BYTES)
            {
                batches.push(std::mem::take(&mut current));
                bytes = 0;
            }
            current.push((index, message));
            bytes += size;
        }
        if !current.is_empty() {
            batches.push(current);
        }
        batches
    }

    /// The `--entries` of `send-message-batch`.
    #[must_use]
    pub fn sqs_entries(batch: &[(usize, &OutboundMessage)]) -> Value {
        Value::Array(
            batch
                .iter()
                .map(|(index, message)| {
                    let mut entry = json!({"Id": index.to_string(), "MessageBody": message.body});
                    if !message.attributes.is_empty() {
                        entry["MessageAttributes"] = attribute_values(&message.attributes);
                    }
                    if let Some(group) = &message.group_id {
                        entry["MessageGroupId"] = json!(group);
                    }
                    if let Some(dedup) = &message.deduplication_id {
                        entry["MessageDeduplicationId"] = json!(dedup);
                    }
                    entry
                })
                .collect(),
        )
    }

    /// The `--publish-batch-request-entries` of `publish-batch`.
    #[must_use]
    pub fn sns_entries(batch: &[(usize, &OutboundMessage)]) -> Value {
        Value::Array(
            batch
                .iter()
                .map(|(index, message)| {
                    let mut entry = json!({"Id": index.to_string(), "Message": message.body});
                    if !message.attributes.is_empty() {
                        entry["MessageAttributes"] = attribute_values(&message.attributes);
                    }
                    if let Some(subject) = &message.subject {
                        entry["Subject"] = json!(subject);
                    }
                    if let Some(group) = &message.group_id {
                        entry["MessageGroupId"] = json!(group);
                    }
                    if let Some(dedup) = &message.deduplication_id {
                        entry["MessageDeduplicationId"] = json!(dedup);
                    }
                    entry
                })
                .collect(),
        )
    }

    /// What a preview shows: each message's index, body cut, attributes, ids.
    #[must_use]
    pub fn preview(&self) -> Vec<Value> {
        self.messages
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let (body, cut) = cut_body(&m.body);
                json!({
                    "index": index,
                    "body": body,
                    "bodyCut": cut,
                    "bytes": m.body.len(),
                    "attributes": m.attributes,
                    "groupId": m.group_id,
                    "deduplicationId": m.deduplication_id,
                    "subject": m.subject,
                })
            })
            .collect()
    }
}

fn attributes_of(raw: Option<&Value>) -> Result<BTreeMap<String, String>, InputError> {
    let Some(raw) = raw.filter(|v| !v.is_null()) else {
        return Ok(BTreeMap::new());
    };
    let object: &Map<String, Value> = raw.as_object().ok_or(InputError::Invalid {
        what: "attributes",
        value: raw.to_string().chars().take(60).collect(),
        rule: "an object of name to text value",
    })?;
    if object.len() > ATTRIBUTES_MAX {
        return Err(InputError::OutOfRange {
            what: "attributes",
            min: 0,
            max: ATTRIBUTES_MAX as i64,
        });
    }
    let mut attributes = BTreeMap::new();
    for (name, value) in object {
        let valid_name = !name.is_empty()
            && name.len() <= 256
            && !name.starts_with('.')
            && !name.ends_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if !valid_name {
            return Err(InputError::Invalid {
                what: "attribute name",
                value: name.chars().take(60).collect(),
                rule: "letters, digits, `_`, `-` and `.`, not starting or ending with `.`",
            });
        }
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => {
                return Err(InputError::Invalid {
                    what: "attribute value",
                    value: name.clone(),
                    rule: "text, a number or a boolean",
                });
            }
        };
        if text.is_empty() {
            return Err(InputError::Invalid {
                what: "attribute value",
                value: name.clone(),
                rule: "not empty",
            });
        }
        attributes.insert(name.clone(), text);
    }
    Ok(attributes)
}

fn attribute_values(attributes: &BTreeMap<String, String>) -> Value {
    Value::Object(
        attributes
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    json!({"DataType": "String", "StringValue": value}),
                )
            })
            .collect(),
    )
}

fn identifier(what: &'static str, value: &str) -> Result<String, InputError> {
    let ok = !value.is_empty()
        && value.len() <= ID_MAX
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!\"#$%&'()*+,./:;<=>?@[]^_`{|}~-".contains(c));
    if ok {
        Ok(value.to_string())
    } else {
        Err(InputError::Invalid {
            what,
            value: value.chars().take(60).collect(),
            rule: "1 to 128 printable ASCII characters",
        })
    }
}

/// A deduplication id from the body, stable while this process runs; the
/// index keeps two equal bodies in one call apart.
fn derived_id(body: &str, index: usize) -> String {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    index.hash(&mut hasher);
    format!("mcp-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(id: &str, body: &str) -> Value {
        json!({"MessageId": id, "ReceiptHandle": "rh+/=", "Body": body, "Attributes": {"SentTimestamp": "1793707200000", "ApproximateReceiveCount": "3", "MessageGroupId": "g1"}, "MessageAttributes": {"eventType": {"DataType": "String", "StringValue": "Created"}, "blob": {"DataType": "Binary", "BinaryValue": "AA=="}}})
    }

    #[test]
    fn received_messages_are_read_cut_in_answers_and_whole_in_evidence() {
        let messages = received_messages(
            &json!({"Messages": [raw("m1", &"x".repeat(5000)), raw("m2", "{\"a\":1}"), {"nope": 1}]}),
        );
        assert_eq!(messages.len(), 2);
        let first = &messages[0];
        assert_eq!(first.sent.as_deref(), Some("2026-11-03T12:00:00Z"));
        assert_eq!(first.receive_count, Some(3));
        assert_eq!(first.attributes["eventType"], "Created");
        assert_eq!(first.attributes["blob"], "<Binary>");
        let shown = first.to_json();
        assert_eq!(shown["bodyCut"], true);
        assert_eq!(shown["body"].as_str().unwrap().len(), 4096);
        assert!(shown.get("receiptHandle").is_none());
        assert_eq!(first.evidence_line()["body"].as_str().unwrap().len(), 5000);
        assert!(received_messages(&Value::Null).is_empty());
        let summary = MessageSummary::of(&messages);
        assert_eq!(summary.count, 2);
        assert_eq!(summary.unique_bodies, 2);
        assert_eq!(summary.groups["g1"], 2);
        assert_eq!(summary.receive_counts["3"], 2);
        assert_eq!(summary.oldest_sent.as_deref(), Some("2026-11-03T12:00:00Z"));
        assert_eq!(summary.to_json()["uniqueBodies"], 2);
        let bare = ReceivedMessage::read(&json!({"MessageId": "b"})).unwrap();
        assert_eq!(MessageSummary::of(&[bare]).groups["-"], 1);
    }

    #[test]
    fn outbound_messages_are_validated_for_fifo_and_standard() {
        let standard = OutboundMessages::parse(
            &json!([{"body": "hello", "attributes": {"eventType": "Created", "n": 2}}, {"body": {"a": 1}}]),
            None,
            false,
            false,
        )
        .unwrap();
        assert_eq!(standard.len(), 2);
        assert!(!standard.is_empty());
        let batches = standard.batches();
        assert_eq!(batches.len(), 1);
        let entries = OutboundMessages::sqs_entries(&batches[0]);
        assert_eq!(
            entries[0]["MessageAttributes"]["eventType"]["StringValue"],
            "Created"
        );
        assert_eq!(entries[0]["MessageAttributes"]["n"]["StringValue"], "2");
        assert_eq!(entries[1]["MessageBody"], "{\"a\":1}");
        assert!(entries[1].get("MessageGroupId").is_none());
        assert_eq!(standard.preview()[0]["bytes"], 5);

        let fifo = OutboundMessages::parse(
            &json!([{"body": "a", "message_deduplication_id": "d1"}, {"body": "a", "message_group_id": "g2", "subject": "s"}]),
            Some("g1"),
            true,
            true,
        )
        .unwrap();
        let sns = OutboundMessages::sns_entries(&fifo.batches()[0]);
        assert_eq!(sns[0]["MessageGroupId"], "g1");
        assert_eq!(sns[0]["MessageDeduplicationId"], "d1");
        assert_eq!(sns[1]["MessageGroupId"], "g2");
        assert_eq!(sns[1]["Subject"], "s");
        assert!(
            sns[1]["MessageDeduplicationId"]
                .as_str()
                .unwrap()
                .starts_with("mcp-")
        );
        assert_ne!(
            derived_id("a", 0),
            derived_id("a", 1),
            "equal bodies in one call stay apart"
        );

        for (raw, default, fifo, cbd) in [
            (json!([]), None, false, false),
            (json!({}), None, false, false),
            (json!([{"body": ""}]), None, false, false),
            (json!([{"attributes": {}}]), None, false, false),
            (
                json!([{"body": "x", "message_group_id": "g"}]),
                None,
                false,
                false,
            ),
            (json!([{"body": "x"}]), Some("g"), false, false),
            (json!([{"body": "x"}]), None, true, true),
            (
                json!([{"body": "x", "message_group_id": "g"}]),
                None,
                true,
                false,
            ),
            (
                json!([{"body": "x", "message_group_id": "g ä"}]),
                None,
                true,
                true,
            ),
            (
                json!([{"body": "x", "attributes": "no"}]),
                None,
                false,
                false,
            ),
            (
                json!([{"body": "x", "attributes": {".bad": "v"}}]),
                None,
                false,
                false,
            ),
            (
                json!([{"body": "x", "attributes": {"k": ""}}]),
                None,
                false,
                false,
            ),
            (
                json!([{"body": "x", "attributes": {"k": [1]}}]),
                None,
                false,
                false,
            ),
        ] {
            assert!(
                OutboundMessages::parse(&raw, default, fifo, cbd).is_err(),
                "{raw}"
            );
        }
        let many: Vec<Value> = (0..101).map(|_| json!({"body": "x"})).collect();
        assert!(OutboundMessages::parse(&Value::Array(many), None, false, false).is_err());
        let eleven: Map<String, Value> = (0..11).map(|i| (format!("a{i}"), json!("v"))).collect();
        assert!(
            OutboundMessages::parse(
                &json!([{"body": "x", "attributes": eleven}]),
                None,
                false,
                false
            )
            .is_err()
        );
        assert!(
            OutboundMessages::parse(
                &json!([{"body": "x".repeat(BODY_MAX_BYTES + 1)}]),
                None,
                false,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn batches_split_by_count_and_by_bytes() {
        let twelve: Vec<Value> = (0..12).map(|i| json!({"body": format!("m{i}")})).collect();
        let messages = OutboundMessages::parse(&Value::Array(twelve), None, false, false).unwrap();
        let batches = messages.batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 10);
        assert_eq!(batches[1][0].0, 10);
        let big = "b".repeat(200_000);
        let heavy = OutboundMessages::parse(
            &json!([{"body": big}, {"body": big}, {"body": "small"}]),
            None,
            false,
            false,
        )
        .unwrap();
        let batches = heavy.batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[1].len(), 2);
    }
}
