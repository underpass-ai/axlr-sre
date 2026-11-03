use jiff::Timestamp;
use serde_json::{Value, json};

const MESSAGE_BYTES: usize = 4_096;

/// The events of a `filter-log-events` answer: time to the second, stream,
/// message bounded to 4 KiB.
#[must_use]
pub fn log_events(answer: &Value) -> Vec<Value> {
    answer["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .map(|e| {
                    let message = e["message"].as_str().unwrap_or_default();
                    let cut = message.len() > MESSAGE_BYTES;
                    let mut end = MESSAGE_BYTES.min(message.len());
                    while !message.is_char_boundary(end) {
                        end -= 1;
                    }
                    json!({
                        "time": e["timestamp"].as_i64().and_then(|ms| Timestamp::from_millisecond(ms).ok()).map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
                        "stream": e["logStreamName"],
                        "message": message[..end].trim_end(),
                        "cut": cut,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The groups of a `describe-log-groups` answer.
#[must_use]
pub fn log_groups(answer: &Value) -> Vec<Value> {
    answer["logGroups"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .map(|g| {
                    json!({
                        "name": g["logGroupName"],
                        "storedBytes": g["storedBytes"],
                        "retentionDays": g["retentionInDays"],
                        "created": g["creationTime"].as_i64().and_then(|ms| Timestamp::from_millisecond(ms).ok()).map(|t| t.strftime("%Y-%m-%d").to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_stamped_and_bounded() {
        let answer = json!({"events": [
            {"timestamp": 1793707200000i64, "logStreamName": "2026/11/03/[$LATEST]abc", "message": "x".repeat(5000)},
            {"timestamp": "bad", "message": "END RequestId: 1\n"}
        ], "searchedLogStreams": [{"logStreamName": "s"}]});
        let events = log_events(&answer);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["time"], "2026-11-03T12:00:00Z");
        assert_eq!(events[0]["cut"], true);
        assert_eq!(events[0]["message"].as_str().unwrap().len(), MESSAGE_BYTES);
        assert_eq!(events[1]["time"], Value::Null);
        assert_eq!(events[1]["message"], "END RequestId: 1");
        assert!(log_events(&json!({})).is_empty());
    }

    #[test]
    fn groups_are_listed_with_their_retention() {
        let answer = json!({"logGroups": [{"logGroupName": "/aws/lambda/f", "storedBytes": 10, "retentionInDays": 14, "creationTime": 1793707200000i64}, {"logGroupName": "/g"}]});
        let groups = log_groups(&answer);
        assert_eq!(groups[0]["name"], "/aws/lambda/f");
        assert_eq!(groups[0]["retentionDays"], 14);
        assert_eq!(groups[0]["created"], "2026-11-03");
        assert_eq!(groups[1]["retentionDays"], Value::Null);
        assert!(log_groups(&json!({})).is_empty());
    }
}
