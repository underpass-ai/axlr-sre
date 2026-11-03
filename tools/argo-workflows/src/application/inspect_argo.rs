use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::domain::ArgoQuery;
use crate::ports::ArgoReader;

/// Answers a query: a listing comes back summarised with its cursor, a
/// detail or the logs come back as Argo gave them.
pub struct InspectArgo {
    reader: Arc<dyn ArgoReader>,
}

impl InspectArgo {
    pub fn new(reader: Arc<dyn ArgoReader>) -> Self {
        Self { reader }
    }

    pub async fn execute(&self, query: ArgoQuery) -> Result<Value> {
        let response = self.reader.read(&query).await?;
        if query.page().is_none() {
            return Ok(response);
        }
        let items = response
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let continuation = response
            .pointer("/metadata/continue")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty());
        let entries: Vec<Value> = items.iter().map(Self::summarize).collect();
        Ok(json!({"items": entries, "continue": continuation, "count": entries.len()}))
    }

    /// What a listing shows of each item: identity, phase, times, schedule.
    pub fn summarize(item: &Value) -> Value {
        json!({
            "name": item.pointer("/metadata/name"),
            "namespace": item.pointer("/metadata/namespace"),
            "createdAt": item.pointer("/metadata/creationTimestamp"),
            "phase": item.pointer("/status/phase"),
            "message": item.pointer("/status/message"),
            "startedAt": item.pointer("/status/startedAt"),
            "finishedAt": item.pointer("/status/finishedAt"),
            "schedule": item.pointer("/spec/schedule"),
            "schedules": item.pointer("/spec/schedules"),
            "suspend": item.pointer("/spec/suspend"),
        })
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::domain::{ListKind, ResourceKind};

    struct FakeReader(Value);

    #[async_trait]
    impl ArgoReader for FakeReader {
        async fn read(&self, _: &ArgoQuery) -> Result<Value> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn a_listing_is_summarised_and_its_cursor_kept() {
        let reader = Arc::new(FakeReader(json!({
            "items": [{"metadata": {"name": "build-1", "namespace": "ci"}, "status": {"phase": "Succeeded"}, "spec": {"huge": "omitted"}}],
            "metadata": {"continue": "page2"}
        })));
        let result = InspectArgo::new(reader)
            .execute(ArgoQuery::list(ListKind::Workflows, "ci".into(), 10, None).unwrap())
            .await
            .unwrap();
        assert_eq!(result["items"][0]["name"], "build-1");
        assert_eq!(result["items"][0]["phase"], "Succeeded");
        assert!(result["items"][0].get("huge").is_none());
        assert_eq!(result["continue"], "page2");
        assert_eq!(result["count"], 1);
    }

    #[tokio::test]
    async fn an_empty_cursor_is_null_and_a_missing_list_is_empty() {
        let reader = Arc::new(FakeReader(json!({"metadata": {"continue": ""}})));
        let result = InspectArgo::new(reader)
            .execute(ArgoQuery::list(ListKind::CronWorkflows, "ci".into(), 10, None).unwrap())
            .await
            .unwrap();
        assert_eq!(result["count"], 0);
        assert!(result["continue"].is_null());
    }

    #[tokio::test]
    async fn a_detail_keeps_the_whole_resource() {
        let reader = Arc::new(FakeReader(
            json!({"metadata": {"name": "build-1"}, "spec": {"entrypoint": "main"}}),
        ));
        let result = InspectArgo::new(reader)
            .execute(ArgoQuery::get(ResourceKind::Workflow, "ci".into(), "build-1".into()).unwrap())
            .await
            .unwrap();
        assert_eq!(result["spec"]["entrypoint"], "main");
    }
}
