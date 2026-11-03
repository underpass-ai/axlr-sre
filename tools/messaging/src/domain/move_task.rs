use serde_json::{Value, json};

/// A message move task (the native redrive) as `list-message-move-tasks`
/// lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveTask {
    pub handle: Option<String>,
    pub status: String,
    pub source_arn: Option<String>,
    pub destination_arn: Option<String>,
    pub moved: Option<u64>,
    pub to_move: Option<u64>,
    pub failure: Option<String>,
    pub started: Option<String>,
}

impl MoveTask {
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.status.as_str(), "RUNNING" | "CANCELLING")
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "handle": self.handle,
            "status": self.status,
            "sourceArn": self.source_arn,
            "destinationArn": self.destination_arn,
            "moved": self.moved,
            "toMove": self.to_move,
            "failure": self.failure,
            "started": self.started,
        })
    }
}

#[must_use]
pub fn move_tasks(answer: &Value) -> Vec<MoveTask> {
    answer["Results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|t| MoveTask {
                    handle: t["TaskHandle"].as_str().map(str::to_string),
                    status: t["Status"].as_str().unwrap_or("?").to_string(),
                    source_arn: t["SourceArn"].as_str().map(str::to_string),
                    destination_arn: t["DestinationArn"].as_str().map(str::to_string),
                    moved: t["ApproximateNumberOfMessagesMoved"].as_u64(),
                    to_move: t["ApproximateNumberOfMessagesToMove"].as_u64(),
                    failure: t["FailureReason"].as_str().map(str::to_string),
                    started: t["StartedTimestamp"]
                        .as_i64()
                        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                        .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_are_read_and_a_running_one_is_known() {
        let tasks = move_tasks(&json!({"Results": [
            {"TaskHandle": "h1", "Status": "RUNNING", "SourceArn": "a", "ApproximateNumberOfMessagesMoved": 3, "ApproximateNumberOfMessagesToMove": 10, "StartedTimestamp": 1793707200000i64},
            {"Status": "COMPLETED", "FailureReason": null}
        ]}));
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].is_running());
        assert_eq!(tasks[0].started.as_deref(), Some("2026-11-03T12:00:00Z"));
        assert_eq!(tasks[0].to_json()["moved"], 3);
        assert!(!tasks[1].is_running());
        assert_eq!(tasks[1].handle, None);
        assert!(move_tasks(&Value::Null).is_empty());
    }
}
