use anyhow::{Result, bail};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::InputError;

const LINE_MAX: usize = 4096;
const PAGE_BYTES_MAX: usize = 262_144;

/// Which way a Loki query walks the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Backward,
    Forward,
}

impl Direction {
    pub fn parse(value: &str) -> Result<Self, InputError> {
        match value.trim() {
            "backward" => Ok(Self::Backward),
            "forward" => Ok(Self::Forward),
            other => Err(InputError::UnknownChoice {
                what: "direction",
                value: other.to_string(),
                allowed: "backward, forward",
            }),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backward => "backward",
            Self::Forward => "forward",
        }
    }
}

struct Line {
    nanos: i128,
    labels: Value,
    text: String,
}

/// A Loki `query_range` answer as a page: the lines newest first (or oldest
/// first, forward), each bounded, the page bounded, and `next`, the `end`
/// (or `start`) that continues it. Lines sharing the boundary nanosecond and
/// not yet seen are lost; the README says so.
pub fn log_page(response: &Value, direction: Direction, limit: i64) -> Result<Value> {
    if response["status"] == "error" {
        bail!(
            "Loki rejected the query: {}",
            response["error"].as_str().unwrap_or("no reason given")
        );
    }
    let mut lines: Vec<Line> = response["data"]["result"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|stream| {
            let labels = stream["stream"].clone();
            stream["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |entry| {
                    Some(Line {
                        nanos: entry[0].as_str()?.parse().ok()?,
                        labels: labels.clone(),
                        text: entry[1].as_str()?.to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    match direction {
        Direction::Backward => lines.sort_by_key(|line| std::cmp::Reverse(line.nanos)),
        Direction::Forward => lines.sort_by_key(|line| line.nanos),
    }
    let total = lines.len();
    let mut bytes = 0;
    let mut shown = Vec::new();
    let mut boundary = None;
    for line in lines
        .into_iter()
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
    {
        if bytes >= PAGE_BYTES_MAX {
            break;
        }
        let (text, cut) = cut(&line.text);
        bytes += text.len();
        boundary = Some(line.nanos);
        shown.push(json!({
            "time": rfc3339(line.nanos),
            "labels": line.labels,
            "line": text,
            "cut": cut,
        }));
    }
    let more = total > shown.len() || i64::try_from(total).unwrap_or(i64::MAX) >= limit;
    let next = match (more, boundary, direction) {
        (true, Some(nanos), Direction::Backward) => json!({"end": rfc3339(nanos)}),
        (true, Some(nanos), Direction::Forward) => json!({"start": rfc3339(nanos + 1)}),
        _ => Value::Null,
    };
    Ok(json!({
        "count": shown.len(),
        "direction": direction.as_str(),
        "truncated": total > shown.len(),
        "lines": shown,
        "next": next,
    }))
}

fn cut(text: &str) -> (String, bool) {
    if text.len() <= LINE_MAX {
        return (text.to_string(), false);
    }
    let mut end = LINE_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn rfc3339(nanos: i128) -> String {
    Timestamp::from_nanosecond(nanos).map_or_else(|_| nanos.to_string(), |t| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Value {
        json!({"status": "success", "data": {"resultType": "streams", "result": [
            {"stream": {"app": "web"}, "values": [["1793700000000000002", "second"], ["1793700000000000000", "oldest"]]},
            {"stream": {"app": "api"}, "values": [["1793700000000000005", "newest"], ["bad", "skipped"]]}
        ]}})
    }

    #[test]
    fn backward_pages_newest_first_and_continue_at_the_oldest_seen() {
        let page = log_page(&response(), Direction::Backward, 2).unwrap();
        assert_eq!(page["count"], 2);
        assert_eq!(page["truncated"], true);
        assert_eq!(page["lines"][0]["line"], "newest");
        assert_eq!(page["lines"][0]["labels"]["app"], "api");
        assert_eq!(page["lines"][0]["time"], "2026-11-03T10:00:00.000000005Z");
        assert_eq!(page["lines"][1]["line"], "second");
        assert_eq!(page["next"]["end"], "2026-11-03T10:00:00.000000002Z");
    }

    #[test]
    fn forward_pages_oldest_first_and_continue_one_nanosecond_after_the_newest() {
        let page = log_page(&response(), Direction::Forward, 3).unwrap();
        assert_eq!(page["lines"][0]["line"], "oldest");
        assert_eq!(page["lines"][2]["line"], "newest");
        assert_eq!(page["truncated"], false);
        assert_eq!(page["next"]["start"], "2026-11-03T10:00:00.000000006Z");
        let last = log_page(&response(), Direction::Forward, 10).unwrap();
        assert!(last["next"].is_null());
    }

    #[test]
    fn long_lines_are_cut_and_an_empty_answer_has_no_next() {
        let long =
            json!({"data": {"result": [{"stream": {}, "values": [["1", "é".repeat(3000)]]}]}});
        let page = log_page(&long, Direction::Backward, 10).unwrap();
        assert_eq!(page["lines"][0]["cut"], true);
        assert!(page["lines"][0]["line"].as_str().unwrap().len() <= LINE_MAX);
        let empty = log_page(&json!({"data": {"result": []}}), Direction::Backward, 10).unwrap();
        assert_eq!(empty["count"], 0);
        assert!(empty["next"].is_null());
        assert!(
            log_page(
                &json!({"status": "error", "error": "parse error at line 1"}),
                Direction::Backward,
                10
            )
            .unwrap_err()
            .to_string()
            .contains("parse error")
        );
        assert!(
            log_page(&json!({"status": "error"}), Direction::Backward, 10)
                .unwrap_err()
                .to_string()
                .contains("no reason")
        );
    }

    #[test]
    fn a_page_stops_at_its_byte_budget() {
        let values: Vec<Value> = (0..100)
            .map(|i| json!([format!("{}", 1_000_000_000 + i), "x".repeat(4000)]))
            .collect();
        let big = json!({"data": {"result": [{"stream": {}, "values": values}]}});
        let page = log_page(&big, Direction::Backward, 1000).unwrap();
        assert!(page["count"].as_u64().unwrap() < 100);
        assert_eq!(page["truncated"], true);
        assert!(!page["next"].is_null());
        assert_eq!(rfc3339(i128::MAX), i128::MAX.to_string());
    }

    #[test]
    fn directions_parse_their_names() {
        assert_eq!(Direction::parse(" forward ").unwrap(), Direction::Forward);
        assert_eq!(Direction::parse("backward").unwrap().as_str(), "backward");
        assert!(matches!(
            Direction::parse("up"),
            Err(InputError::UnknownChoice {
                what: "direction",
                ..
            })
        ));
    }
}
