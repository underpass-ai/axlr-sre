use std::fmt;

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::InputError;

pub const NOTE_MAX: usize = 25_000;
const MESSAGE_CHARS: usize = 300;
const DESCRIPTION_CHARS: usize = 2_000;
const DETAILS_MAX: usize = 50;

/// Opsgenie's priority, P1 the most urgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
    P5,
}

impl Priority {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_uppercase().as_str() {
            "P1" => Some(Self::P1),
            "P2" => Some(Self::P2),
            "P3" => Some(Self::P3),
            "P4" => Some(Self::P4),
            "P5" => Some(Self::P5),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
            Self::P5 => "P5",
        }
    }

    /// The incident severity axlr-bots takes: P1 sev1, P2 sev2, P3 sev3, P4 and P5 sev4.
    #[must_use]
    pub fn severity(self) -> &'static str {
        match self {
            Self::P1 => "sev1",
            Self::P2 => "sev2",
            Self::P3 => "sev3",
            Self::P4 | Self::P5 => "sev4",
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A responder as the alerts carry it: a type (`team`, `escalation`,
/// `schedule`, `user`) and an id. Names come from the directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Responder {
    pub kind: String,
    pub id: String,
}

/// An alert as `/v2/alerts` and `/v2/alerts/{id}` answer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub id: String,
    pub tiny_id: String,
    pub message: String,
    pub status: String,
    pub acknowledged: bool,
    pub priority: Priority,
    pub count: u64,
    pub created_at: Option<String>,
    pub last_occurred_at: Option<String>,
    pub source: Option<String>,
    pub alias: Option<String>,
    pub tags: Vec<String>,
    pub responders: Vec<Responder>,
    pub description: Option<String>,
    pub details: Map<String, Value>,
}

impl Alert {
    /// `None` when the item lacks what every alert has: id, tinyId, message.
    pub fn read(item: &Value) -> Option<Self> {
        let text = |key: &str| item[key].as_str().map(str::to_string);
        Some(Self {
            id: text("id")?,
            tiny_id: text("tinyId")?,
            message: text("message")?,
            status: text("status").unwrap_or_else(|| "?".into()),
            acknowledged: item["acknowledged"].as_bool().unwrap_or(false),
            priority: item["priority"]
                .as_str()
                .and_then(Priority::parse)
                .unwrap_or(Priority::P5),
            count: item["count"].as_u64().unwrap_or(1),
            created_at: text("createdAt"),
            last_occurred_at: text("lastOccurredAt"),
            source: text("source"),
            alias: text("alias").filter(|alias| !alias.is_empty()),
            tags: item["tags"]
                .as_array()
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| tag.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            responders: item["responders"]
                .as_array()
                .map(|responders| {
                    responders
                        .iter()
                        .filter_map(|r| {
                            Some(Responder {
                                kind: r["type"].as_str()?.to_string(),
                                id: r["id"].as_str()?.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            description: text("description").filter(|d| !d.trim().is_empty()),
            details: item["details"].as_object().cloned().unwrap_or_default(),
        })
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == "open"
    }

    /// The message on one line, cut with an ellipsis past `max_chars`.
    #[must_use]
    pub fn one_line_message(&self, max_chars: usize) -> String {
        one_line(&self.message, max_chars)
    }

    /// `createdAt` to the minute, UTC.
    #[must_use]
    pub fn created_label(&self) -> String {
        self.created_at
            .as_deref()
            .map(to_minute)
            .unwrap_or_else(|| "?".into())
    }

    /// The row `alerts_open` shows.
    #[must_use]
    pub fn summary(&self, via: &str) -> Value {
        json!({
            "tinyId": self.tiny_id,
            "id": self.id,
            "message": self.one_line_message(MESSAGE_CHARS),
            "priority": self.priority.as_str(),
            "status": self.status,
            "acknowledged": self.acknowledged,
            "count": self.count,
            "created": self.created_label(),
            "via": via,
        })
    }

    /// What `alert` shows: the summary plus the description, the details,
    /// the tags, the source and every responder with its name when known.
    #[must_use]
    pub fn detail(&self, via: &str, responder_names: &[(String, String)]) -> Value {
        let mut value = self.summary(via);
        let description = self.description.as_deref().map(|d| {
            let (text, cut) = bounded(d, DESCRIPTION_CHARS);
            json!({"text": text, "cut": cut})
        });
        let details: Map<String, Value> = self
            .details
            .iter()
            .take(DETAILS_MAX)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        value["description"] = description.unwrap_or(Value::Null);
        value["details"] = Value::Object(details);
        value["detailsTruncated"] = json!(self.details.len() > DETAILS_MAX);
        value["tags"] = json!(self.tags);
        value["source"] = json!(self.source);
        value["alias"] = json!(self.alias);
        value["lastOccurred"] = json!(self.last_occurred_at.as_deref().map(to_minute));
        value["responders"] = self
            .responders
            .iter()
            .map(|r| {
                let name = responder_names
                    .iter()
                    .find(|(id, _)| *id == r.id)
                    .map(|(_, name)| name.clone());
                json!({"type": r.kind, "id": r.id, "name": name})
            })
            .collect();
        value
    }
}

fn one_line(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let mut cut: String = flat.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn bounded(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        (text.to_string(), false)
    } else {
        (text.chars().take(max_chars).collect(), true)
    }
}

fn to_minute(stamp: &str) -> String {
    stamp
        .parse::<Timestamp>()
        .map(|t| t.strftime("%Y-%m-%dT%H:%MZ").to_string())
        .unwrap_or_else(|_| stamp.to_string())
}

/// How a call names an alert: the tiny id (`123` or `#123`) or the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertRef {
    Tiny(String),
    Id(String),
}

impl AlertRef {
    pub fn parse(raw: &str) -> Result<Self, InputError> {
        let raw = raw.trim();
        let tiny = raw.strip_prefix('#').unwrap_or(raw);
        if !tiny.is_empty() && tiny.len() <= 12 && tiny.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Self::Tiny(tiny.to_string()));
        }
        if super::scope::looks_like_id(raw) {
            return Ok(Self::Id(raw.to_string()));
        }
        Err(InputError::InvalidAlert(raw.chars().take(60).collect()))
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        match self {
            Self::Tiny(tiny) | Self::Id(tiny) => tiny,
        }
    }

    #[must_use]
    pub fn identifier_type(&self) -> &'static str {
        match self {
            Self::Tiny(_) => "tiny",
            Self::Id(_) => "id",
        }
    }

    /// Whether this reference names the alert.
    #[must_use]
    pub fn matches(&self, alert: &Alert) -> bool {
        match self {
            Self::Tiny(tiny) => alert.tiny_id == *tiny,
            Self::Id(id) => alert.id == *id,
        }
    }
}

impl fmt::Display for AlertRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tiny(tiny) => write!(f, "#{tiny}"),
            Self::Id(id) => f.write_str(id),
        }
    }
}

/// A note for an alert: what was done and why, 1 to 25 000 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note(String);

impl Note {
    pub fn parse(raw: Option<&str>) -> Result<Self, InputError> {
        let text = raw.map(str::trim).unwrap_or_default();
        if text.is_empty() {
            return Err(InputError::Empty("note"));
        }
        if text.chars().count() > NOTE_MAX {
            return Err(InputError::TooLong {
                what: "note",
                max: NOTE_MAX,
            });
        }
        Ok(Self(text.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An open alert of the scope and which team or escalation let it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedAlert {
    pub alert: Alert,
    pub via: String,
}

/// The open alerts of the account, split by the scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenAlerts {
    pub alerts: Vec<ScopedAlert>,
    pub excluded: usize,
    pub unreadable: usize,
    /// False when Opsgenie had more pages than were read: the list is not
    /// the whole truth, so nothing may be assumed closed from it.
    pub complete: bool,
}

impl OpenAlerts {
    #[must_use]
    pub fn find(&self, reference: &AlertRef) -> Option<&ScopedAlert> {
        self.alerts
            .iter()
            .find(|scoped| reference.matches(&scoped.alert))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(tiny: u32, priority: &str) -> Value {
        json!({
            "id": format!("00000000-0000-4000-8000-{tiny:012}"),
            "tinyId": tiny.to_string(),
            "message": format!("Disk  almost\nfull on node-{tiny}"),
            "status": "open",
            "acknowledged": false,
            "priority": priority,
            "count": 3,
            "createdAt": "2026-11-03T08:15:42.064Z",
            "lastOccurredAt": "2026-11-03T09:00:00.000Z",
            "source": "prometheus",
            "alias": "",
            "tags": ["disk"],
            "responders": [{"type": "team", "id": "team-1"}, {"type": "escalation", "id": "esc-1"}, {"type": "user"}],
        })
    }

    #[test]
    fn priorities_map_to_severities() {
        for (raw, severity) in [
            ("p1", "sev1"),
            ("P2", "sev2"),
            ("P3", "sev3"),
            ("P4", "sev4"),
            ("P5", "sev4"),
        ] {
            let priority = Priority::parse(raw).unwrap();
            assert_eq!(priority.severity(), severity);
            assert_eq!(priority.to_string(), raw.to_ascii_uppercase());
        }
        assert_eq!(Priority::parse("P6"), None);
    }

    #[test]
    fn an_alert_is_read_summarised_and_detailed() {
        let alert = Alert::read(&sample(7, "P2")).unwrap();
        assert!(alert.is_open());
        assert_eq!(alert.responders.len(), 2);
        assert_eq!(alert.alias, None);
        assert_eq!(alert.one_line_message(100), "Disk almost full on node-7");
        assert_eq!(alert.one_line_message(10), "Disk almo…");
        assert_eq!(alert.created_label(), "2026-11-03T08:15Z");
        let summary = alert.summary("Platform");
        assert_eq!(summary["tinyId"], "7");
        assert_eq!(summary["priority"], "P2");
        assert_eq!(summary["via"], "Platform");
        let names = vec![("team-1".to_string(), "Platform".to_string())];
        let detail = alert.detail("Platform", &names);
        assert_eq!(detail["responders"][0]["name"], "Platform");
        assert_eq!(detail["responders"][1]["name"], Value::Null);
        assert_eq!(detail["description"], Value::Null);
        assert_eq!(detail["lastOccurred"], "2026-11-03T09:00Z");
        assert_eq!(detail["tags"][0], "disk");

        let mut raw = sample(8, "weird");
        raw["description"] = json!("d".repeat(3000));
        raw["details"] = json!({"host": "node-8"});
        raw["createdAt"] = json!("not a date");
        let alert = Alert::read(&raw).unwrap();
        assert_eq!(alert.priority, Priority::P5);
        assert_eq!(alert.created_label(), "not a date");
        let detail = alert.detail("x", &[]);
        assert_eq!(detail["description"]["cut"], true);
        assert_eq!(detail["description"]["text"].as_str().unwrap().len(), 2000);
        assert_eq!(detail["details"]["host"], "node-8");
        assert_eq!(detail["detailsTruncated"], false);
        assert!(Alert::read(&json!({"id": "x"})).is_none());
        let bare = Alert::read(&json!({"id": "x", "tinyId": "1", "message": "m"})).unwrap();
        assert_eq!(bare.status, "?");
        assert!(!bare.is_open());
        assert_eq!(bare.created_label(), "?");
    }

    #[test]
    fn references_are_tiny_ids_or_ids_and_notes_are_bounded() {
        assert_eq!(
            AlertRef::parse(" #123 ").unwrap(),
            AlertRef::Tiny("123".into())
        );
        assert_eq!(AlertRef::parse("123").unwrap().identifier_type(), "tiny");
        let id = AlertRef::parse("00000000-0000-4000-8000-000000000007").unwrap();
        assert_eq!(id.identifier_type(), "id");
        assert_eq!(id.identifier(), "00000000-0000-4000-8000-000000000007");
        assert_eq!(id.to_string(), "00000000-0000-4000-8000-000000000007");
        assert_eq!(AlertRef::parse("#9").unwrap().to_string(), "#9");
        for bad in ["", "#", "abc", "1234567890123", "x; rm"] {
            assert!(matches!(
                AlertRef::parse(bad).unwrap_err(),
                InputError::InvalidAlert(_)
            ));
        }
        let alert = Alert::read(&sample(7, "P1")).unwrap();
        assert!(AlertRef::Tiny("7".into()).matches(&alert));
        assert!(id.matches(&alert));
        assert!(!AlertRef::Tiny("8".into()).matches(&alert));
        let open = OpenAlerts {
            alerts: vec![ScopedAlert {
                alert,
                via: "t".into(),
            }],
            ..OpenAlerts::default()
        };
        assert!(open.find(&AlertRef::Tiny("7".into())).is_some());
        assert!(open.find(&AlertRef::Tiny("8".into())).is_none());

        assert_eq!(Note::parse(Some("  fixed  ")).unwrap().as_str(), "fixed");
        assert_eq!(Note::parse(None).unwrap_err(), InputError::Empty("note"));
        assert_eq!(
            Note::parse(Some(" ")).unwrap_err(),
            InputError::Empty("note")
        );
        assert!(matches!(
            Note::parse(Some(&"n".repeat(NOTE_MAX + 1))).unwrap_err(),
            InputError::TooLong { what: "note", .. }
        ));
    }
}
