use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::domain::{ArgoQuery, ListKind};
use crate::ports::ArgoReader;

const PAGE: u16 = 100;

/// `discover`: what the configured namespace holds, bounded to one page per
/// kind: workflows by phase, templates and cron workflows by name.
pub struct DiscoverScope {
    reader: Arc<dyn ArgoReader>,
    namespace: String,
}

impl DiscoverScope {
    pub fn new(reader: Arc<dyn ArgoReader>, namespace: String) -> Self {
        Self { reader, namespace }
    }

    pub async fn execute(&self, namespace: Option<String>) -> Result<Value> {
        let namespace = namespace.unwrap_or_else(|| self.namespace.clone());
        let workflows = self.page(ListKind::Workflows, &namespace).await?;
        let mut by_phase: BTreeMap<String, usize> = BTreeMap::new();
        for item in &workflows.0 {
            let phase = item
                .pointer("/status/phase")
                .and_then(Value::as_str)
                .unwrap_or("Unknown");
            *by_phase.entry(phase.to_string()).or_default() += 1;
        }
        let templates = self.page(ListKind::WorkflowTemplates, &namespace).await?;
        let crons = self.page(ListKind::CronWorkflows, &namespace).await?;
        Ok(json!({
            "namespace": namespace,
            "workflows": {"count": workflows.0.len(), "byPhase": by_phase, "more": workflows.1},
            "workflowTemplates": {"count": templates.0.len(), "names": names(&templates.0), "more": templates.1},
            "cronWorkflows": {"count": crons.0.len(), "names": names(&crons.0), "more": crons.1},
            "readOnly": true,
        }))
    }

    async fn page(&self, kind: ListKind, namespace: &str) -> Result<(Vec<Value>, bool)> {
        let query = ArgoQuery::list(kind, namespace.to_string(), PAGE, None)?;
        let response = self.reader.read(&query).await?;
        let items = response
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let more = response
            .pointer("/metadata/continue")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty());
        Ok((items, more))
    }
}

fn names(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter_map(|item| item.pointer("/metadata/name").cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    struct Reader;

    #[async_trait]
    impl ArgoReader for Reader {
        async fn read(&self, query: &ArgoQuery) -> Result<Value> {
            Ok(match query {
                ArgoQuery::List {
                    kind: ListKind::Workflows,
                    ..
                } => json!({
                    "items": [
                        {"metadata": {"name": "a"}, "status": {"phase": "Failed"}},
                        {"metadata": {"name": "b"}, "status": {"phase": "Succeeded"}},
                        {"metadata": {"name": "c"}},
                    ],
                    "metadata": {"continue": "more"}
                }),
                ArgoQuery::List {
                    kind: ListKind::WorkflowTemplates,
                    ..
                } => json!({"items": [{"metadata": {"name": "build"}}]}),
                _ => json!({"items": []}),
            })
        }
    }

    #[tokio::test]
    async fn counts_workflows_by_phase_and_names_the_rest() {
        let scope = DiscoverScope::new(Arc::new(Reader), "ci".into())
            .execute(None)
            .await
            .unwrap();
        assert_eq!(scope["namespace"], "ci");
        assert_eq!(scope["workflows"]["count"], 3);
        assert_eq!(
            scope["workflows"]["byPhase"],
            json!({"Failed": 1, "Succeeded": 1, "Unknown": 1})
        );
        assert_eq!(scope["workflows"]["more"], true);
        assert_eq!(scope["workflowTemplates"]["names"], json!(["build"]));
        assert_eq!(scope["cronWorkflows"]["count"], 0);
        assert_eq!(scope["cronWorkflows"]["more"], false);
    }

    #[tokio::test]
    async fn a_namespace_given_wins_and_is_validated() {
        let scope = DiscoverScope::new(Arc::new(Reader), "ci".into());
        assert_eq!(
            scope.execute(Some("other".into())).await.unwrap()["namespace"],
            "other"
        );
        assert!(scope.execute(Some("Bad".into())).await.is_err());
    }
}
