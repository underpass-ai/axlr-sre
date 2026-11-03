use std::collections::BTreeMap;

use serde_json::{Value, json};

const TEXT_SHOWN: usize = 4_096;

/// A RESP2 reply, as the driver gives it, without the driver's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Nil,
    Int(i64),
    Text(String),
    /// Bytes that are not UTF-8, as hex.
    Bytes(String),
    Array(Vec<Reply>),
}

impl Reply {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Text(text) => text.parse().ok(),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Reply]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// A text or an integer as text; nil as `None`.
    #[must_use]
    pub fn to_text(&self) -> Option<String> {
        match self {
            Self::Nil => None,
            Self::Int(n) => Some(n.to_string()),
            Self::Text(text) => Some(text.clone()),
            Self::Bytes(hex) => Some(format!("<bytes {hex}>")),
            Self::Array(_) => Some(self.to_json().to_string()),
        }
    }

    /// JSON for an answer: texts cut at 4 KiB.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Nil => Value::Null,
            Self::Int(n) => json!(n),
            Self::Text(text) => json!(cut(text)),
            Self::Bytes(hex) => json!(format!("<bytes {}>", cut(hex))),
            Self::Array(items) => Value::Array(items.iter().map(Reply::to_json).collect()),
        }
    }
}

fn cut(text: &str) -> String {
    if text.len() <= TEXT_SHOWN {
        return text.to_string();
    }
    let mut end = TEXT_SHOWN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// An even-length array of alternating names and values (CONFIG GET,
/// HGETALL, MEMORY STATS under RESP2) as a map.
#[must_use]
pub fn pairs_to_map(reply: &Reply) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    if let Some(items) = reply.as_array() {
        for pair in items.chunks(2) {
            if let [name, value] = pair
                && let Some(name) = name.to_text()
            {
                map.insert(name, value.to_json());
            }
        }
    }
    map
}

/// `INFO`: `# Section` headers and `key:value` lines, by section.
#[must_use]
pub fn info_sections(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections = BTreeMap::new();
    let mut current = String::from("other");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('#') {
            current = name.trim().to_ascii_lowercase();
            sections
                .entry(current.clone())
                .or_insert_with(BTreeMap::new);
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            sections
                .entry(current.clone())
                .or_insert_with(BTreeMap::new)
                .insert(key.to_string(), value.to_string());
        }
    }
    sections
}

/// `CLIENT LIST`: one client per line, `name=value` pairs.
#[must_use]
pub fn client_list(text: &str) -> Vec<BTreeMap<String, String>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_whitespace()
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}

/// `SLOWLOG GET`: id, time, microseconds, the command (joined, cut), and
/// the client's address and name when the server gives them.
#[must_use]
pub fn slowlog(reply: &Reply) -> Vec<Value> {
    reply
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let fields = entry.as_array()?;
                    let command: Vec<String> = fields
                        .get(3)
                        .and_then(Reply::as_array)
                        .map(|parts| parts.iter().filter_map(Reply::to_text).collect())
                        .unwrap_or_default();
                    Some(json!({
                        "id": fields.first().and_then(Reply::as_int),
                        "time": fields.get(1).and_then(Reply::as_int),
                        "microseconds": fields.get(2).and_then(Reply::as_int),
                        "command": cut(&command.join(" ")),
                        "client": fields.get(4).and_then(Reply::to_text),
                        "clientName": fields.get(5).and_then(Reply::to_text).filter(|n| !n.is_empty()),
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `LATENCY LATEST`: event, time, latest and max milliseconds.
#[must_use]
pub fn latency_latest(reply: &Reply) -> Vec<Value> {
    reply
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter_map(|event| {
                    let fields = event.as_array()?;
                    Some(json!({
                        "event": fields.first().and_then(Reply::to_text),
                        "time": fields.get(1).and_then(Reply::as_int),
                        "latestMs": fields.get(2).and_then(Reply::as_int),
                        "maxMs": fields.get(3).and_then(Reply::as_int),
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `CLUSTER NODES`: id, address, flags, primary, slots per line.
#[must_use]
pub fn cluster_nodes(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            json!({
                "id": fields.first(),
                "address": fields.get(1),
                "flags": fields.get(2),
                "primary": fields.get(3).filter(|p| **p != "-"),
                "linkState": fields.get(7),
                "slots": fields.get(8..).map(|s| s.join(" ")).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Reply {
        Reply::Text(s.into())
    }

    #[test]
    fn replies_convert_and_pairs_become_a_map() {
        assert_eq!(Reply::Int(3).to_text().as_deref(), Some("3"));
        assert_eq!(Reply::Nil.to_text(), None);
        assert_eq!(text("7").as_int(), Some(7));
        assert_eq!(
            Reply::Bytes("ff".into()).to_text().as_deref(),
            Some("<bytes ff>")
        );
        assert_eq!(
            Reply::Array(vec![Reply::Int(1), Reply::Nil]).to_json(),
            json!([1, null])
        );
        assert_eq!(
            Reply::Array(vec![text("a")]).to_text().as_deref(),
            Some("[\"a\"]")
        );
        let long = text(&"x".repeat(5000));
        assert!(long.to_json().as_str().unwrap().ends_with('…'));
        let map = pairs_to_map(&Reply::Array(vec![
            text("maxmemory"),
            text("0"),
            text("policy"),
            Reply::Int(1),
            text("odd"),
        ]));
        assert_eq!(map["maxmemory"], "0");
        assert_eq!(map["policy"], 1);
        assert_eq!(map.len(), 2);
        assert!(pairs_to_map(&Reply::Nil).is_empty());
    }

    #[test]
    fn info_client_list_slowlog_latency_and_cluster_nodes_are_parsed() {
        let info = info_sections(
            "# Server\r\nredis_version:7.2.4\r\n\r\n# Memory\r\nused_memory:1024\r\nmaxmemory:0\r\nloose:line\r\n",
        );
        assert_eq!(info["server"]["redis_version"], "7.2.4");
        assert_eq!(info["memory"]["used_memory"], "1024");
        assert!(!info.contains_key("other"));
        let clients = client_list(
            "id=3 addr=127.0.0.1:50000 name= flags=N cmd=client|list\nid=4 addr=127.0.0.1:50001 flags=S cmd=replconf\n",
        );
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[1]["flags"], "S");
        assert_eq!(clients[0]["name"], "");
        let entries = slowlog(&Reply::Array(vec![
            Reply::Array(vec![
                Reply::Int(14),
                Reply::Int(1793707200),
                Reply::Int(12345),
                Reply::Array(vec![text("SCAN"), text("0"), text("MATCH"), text("demo:*")]),
                text("127.0.0.1:50000"),
                text(""),
            ]),
            Reply::Int(9),
        ]));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["command"], "SCAN 0 MATCH demo:*");
        assert_eq!(entries[0]["microseconds"], 12345);
        assert_eq!(entries[0]["clientName"], Value::Null);
        let latency = latency_latest(&Reply::Array(vec![Reply::Array(vec![
            text("command"),
            Reply::Int(1793707200),
            Reply::Int(250),
            Reply::Int(1000),
        ])]));
        assert_eq!(latency[0]["event"], "command");
        assert_eq!(latency[0]["maxMs"], 1000);
        assert!(latency_latest(&Reply::Nil).is_empty());
        let nodes = cluster_nodes(
            "07c3 127.0.0.1:30004@40004 slave e7d1 0 1426238317239 4 connected\ne7d1 127.0.0.1:30001@40001 myself,master - 0 0 1 connected 0-5460\n",
        );
        assert_eq!(nodes[0]["primary"], "e7d1");
        assert_eq!(nodes[1]["primary"], Value::Null);
        assert_eq!(nodes[1]["slots"], "0-5460");
        assert_eq!(nodes[1]["flags"], "myself,master");
    }
}
