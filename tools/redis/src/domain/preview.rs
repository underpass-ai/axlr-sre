use serde_json::{Value, json};

use super::Reply;

/// How many elements of a collection a preview reads.
pub const PREVIEW_ELEMENTS: usize = 50;
const PREVIEW_BYTES: usize = 65_536;

/// The command that previews a value of a given type: a bounded slice,
/// never the whole collection. `None` for a type with no preview.
#[must_use]
pub fn preview_command(kind: &str, key: &str) -> Option<Vec<String>> {
    let n = PREVIEW_ELEMENTS.to_string();
    let last = (PREVIEW_ELEMENTS - 1).to_string();
    let command: Vec<&str> = match kind {
        "string" => vec!["GETRANGE", key, "0", "65535"],
        "list" => vec!["LRANGE", key, "0", &last],
        "set" => vec!["SSCAN", key, "0", "COUNT", &n],
        "zset" => vec!["ZRANGE", key, "0", &last, "WITHSCORES"],
        "hash" => vec!["HSCAN", key, "0", "COUNT", &n],
        "stream" => vec!["XRANGE", key, "-", "+", "COUNT", &n],
        _ => return None,
    };
    Some(command.into_iter().map(str::to_string).collect())
}

/// A value's preview: text to match a value filter against, and JSON to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub kind: String,
    pub text: String,
    pub shown: Value,
    pub partial: bool,
}

impl Preview {
    /// From the reply of `preview_command`. SSCAN and HSCAN answer
    /// `[cursor, items]`; a cursor other than 0 means more elements exist.
    #[must_use]
    pub fn of(kind: &str, reply: &Reply) -> Self {
        let (items, partial) = match kind {
            "set" | "hash" => {
                let cursor = reply
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Reply::as_int)
                    .unwrap_or(0);
                let items = reply
                    .as_array()
                    .and_then(|a| a.get(1))
                    .and_then(Reply::as_array)
                    .map(<[Reply]>::to_vec)
                    .unwrap_or_default();
                (items, cursor != 0)
            }
            "string" => {
                let text = reply.to_text().unwrap_or_default();
                let partial = text.len() >= PREVIEW_BYTES;
                return Self {
                    kind: kind.into(),
                    shown: json!(cut(&text)),
                    text,
                    partial,
                };
            }
            _ => {
                let items = reply.as_array().map(<[Reply]>::to_vec).unwrap_or_default();
                let count = match kind {
                    "zset" => items.len() / 2,
                    _ => items.len(),
                };
                (items, count >= PREVIEW_ELEMENTS)
            }
        };
        let shown: Value = match kind {
            "hash" => Value::Object(
                items
                    .chunks(2)
                    .filter_map(|pair| match pair {
                        [k, v] => Some((k.to_text().unwrap_or_default(), v.to_json())),
                        _ => None,
                    })
                    .collect(),
            ),
            "zset" => Value::Array(
                items
                    .chunks(2)
                    .filter_map(|pair| match pair {
                        [member, score] => {
                            Some(json!({"member": member.to_json(), "score": score.to_text()}))
                        }
                        _ => None,
                    })
                    .collect(),
            ),
            _ => Value::Array(items.iter().map(Reply::to_json).collect()),
        };
        let text: String = items
            .iter()
            .filter_map(Reply::to_text)
            .collect::<Vec<_>>()
            .join(" ");
        let text: String = text.chars().take(PREVIEW_BYTES).collect();
        Self {
            kind: kind.into(),
            text,
            shown,
            partial,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"type": self.kind, "value": self.shown, "partial": self.partial})
    }
}

fn cut(text: &str) -> String {
    if text.len() <= 4_096 {
        return text.to_string();
    }
    let mut end = 4_096;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Reply {
        Reply::Text(s.into())
    }

    #[test]
    fn each_type_has_a_bounded_preview_command() {
        assert_eq!(
            preview_command("string", "k").unwrap(),
            ["GETRANGE", "k", "0", "65535"]
        );
        assert_eq!(preview_command("list", "k").unwrap()[3], "49");
        assert_eq!(preview_command("set", "k").unwrap()[0], "SSCAN");
        assert_eq!(preview_command("zset", "k").unwrap()[4], "WITHSCORES");
        assert_eq!(preview_command("hash", "k").unwrap()[0], "HSCAN");
        assert_eq!(preview_command("stream", "k").unwrap()[5], "50");
        assert_eq!(preview_command("none", "k"), None);
        assert_eq!(preview_command("ReJSON-RL", "k"), None);
    }

    #[test]
    fn previews_are_built_per_type_with_a_partial_flag() {
        let string = Preview::of("string", &text("hello"));
        assert_eq!(string.text, "hello");
        assert!(!string.partial);
        let hash = Preview::of(
            "hash",
            &Reply::Array(vec![
                Reply::Int(12),
                Reply::Array(vec![text("f"), text("v")]),
            ]),
        );
        assert_eq!(hash.shown["f"], "v");
        assert!(hash.partial, "a cursor other than 0 means more");
        assert_eq!(hash.text, "f v");
        let zset = Preview::of("zset", &Reply::Array(vec![text("a"), text("1.5")]));
        assert_eq!(zset.shown[0]["score"], "1.5");
        assert!(!zset.partial);
        let full_list = Preview::of(
            "list",
            &Reply::Array(
                (0..PREVIEW_ELEMENTS)
                    .map(|i| Reply::Int(i as i64))
                    .collect(),
            ),
        );
        assert!(full_list.partial);
        assert_eq!(full_list.shown.as_array().unwrap().len(), PREVIEW_ELEMENTS);
        assert_eq!(full_list.to_json()["type"], "list");
        let set = Preview::of(
            "set",
            &Reply::Array(vec![Reply::Int(0), Reply::Array(vec![text("m")])]),
        );
        assert_eq!(set.shown, json!(["m"]));
        assert!(!set.partial);
        let big = Preview::of("string", &text(&"x".repeat(PREVIEW_BYTES)));
        assert!(big.partial);
        assert!(big.shown.as_str().unwrap().ends_with('…'));
    }
}
