use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::ReadAlerts;
use crate::domain::Priority;

/// `discover`: the scope with its ids, how many alerts are open in it by
/// priority and how many are outside, what the tools read and what they
/// write. One read of the open alerts, nothing else.
pub struct DiscoverScope {
    read: Arc<ReadAlerts>,
    server: String,
    writes: bool,
}

impl DiscoverScope {
    pub fn new(read: Arc<ReadAlerts>, server: String, writes: bool) -> Self {
        Self {
            read,
            server,
            writes,
        }
    }

    pub async fn execute(&self) -> Result<Value> {
        let scope = self.read.resolve_scope().await?;
        let open = self.read.open().await?;
        let mut by_priority: BTreeMap<&str, usize> = BTreeMap::new();
        let mut acknowledged = 0;
        for scoped in &open.alerts {
            *by_priority
                .entry(scoped.alert.priority.as_str())
                .or_default() += 1;
            if scoped.alert.acknowledged {
                acknowledged += 1;
            }
        }
        let priorities: Vec<Value> = [
            Priority::P1,
            Priority::P2,
            Priority::P3,
            Priority::P4,
            Priority::P5,
        ]
        .iter()
        .filter_map(|p| {
            by_priority
                .get(p.as_str())
                .map(|n| json!({"priority": p.as_str(), "open": n}))
        })
        .collect();
        Ok(json!({
            "server": self.server,
            "scope": scope.to_json(),
            "open": {
                "inScope": open.alerts.len(),
                "acknowledged": acknowledged,
                "byPriority": priorities,
                "excluded": open.excluded,
                "complete": open.complete,
            },
            "reads": ["alerts_open", "alert", "alert_notes"],
            "writes": if self.writes { json!(["alert_acknowledge", "alert_note", "alert_close"]) } else { json!([]) },
            "writesHint": if self.writes { "each write previews unless confirm=true" } else { "register with OPSGENIE_WRITES=true to publish acknowledge, note and close" },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeClient, listings, ours, page, theirs};
    use super::*;
    use crate::domain::Scope;
    use crate::ports::OpsgenieClient;

    #[tokio::test]
    async fn the_scope_and_the_open_counts_are_shown() {
        let mut answers = listings();
        let mut acked = ours(2, "P1");
        acked["acknowledged"] = json!(true);
        answers.push(page(vec![ours(1, "P1"), acked, ours(3, "P4"), theirs(4)]));
        let client: Arc<dyn OpsgenieClient> = FakeClient::answering(answers);
        let read = Arc::new(ReadAlerts::new(
            client,
            Scope::new(Some("platform"), Some("critical")).unwrap(),
        ));
        let answer = DiscoverScope::new(read.clone(), "https://api.example/".into(), false)
            .execute()
            .await
            .unwrap();
        assert_eq!(answer["scope"]["teams"][0]["name"], "Platform");
        assert_eq!(answer["scope"]["escalations"][0]["name"], "Critical");
        assert_eq!(answer["open"]["inScope"], 3);
        assert_eq!(answer["open"]["acknowledged"], 1);
        assert_eq!(answer["open"]["excluded"], 1);
        assert_eq!(
            answer["open"]["byPriority"],
            json!([{"priority": "P1", "open": 2}, {"priority": "P4", "open": 1}])
        );
        assert_eq!(answer["writes"], json!([]));
        assert!(
            answer["writesHint"]
                .as_str()
                .unwrap()
                .contains("OPSGENIE_WRITES")
        );
        let with_writes = DiscoverScope::new(read, "s".into(), true)
            .execute()
            .await
            .unwrap();
        assert_eq!(with_writes["writes"].as_array().unwrap().len(), 3);
    }
}
