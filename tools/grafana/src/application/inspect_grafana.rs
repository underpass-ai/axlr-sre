use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::domain::{Request, alerts, dashboard_detail, dashboard_summaries};
use crate::ports::GrafanaReader;

/// Grafana's own objects: the dashboards and the alerts.
pub struct InspectGrafana {
    reader: Arc<dyn GrafanaReader>,
}

impl InspectGrafana {
    pub fn new(reader: Arc<dyn GrafanaReader>) -> Self {
        Self { reader }
    }

    pub async fn dashboards(
        &self,
        text: Option<&str>,
        tag: Option<&str>,
        limit: i64,
    ) -> Result<Value> {
        let response = self
            .reader
            .get(&Request::search_dashboards(text, tag, limit)?)
            .await?;
        let items = dashboard_summaries(&response);
        Ok(
            json!({"count": items.len(), "more": i64::try_from(items.len()).unwrap_or(i64::MAX) >= limit, "dashboards": items}),
        )
    }

    pub async fn dashboard(&self, uid: &str, with_queries: bool) -> Result<Value> {
        let response = self.reader.get(&Request::dashboard(uid)?).await?;
        Ok(dashboard_detail(&response, with_queries))
    }

    /// Grafana's own Alertmanager unless a datasource uid is named.
    pub async fn alerts(&self, datasource: Option<&str>, firing_only: bool) -> Result<Value> {
        let uid = datasource
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("grafana");
        let response = self.reader.get(&Request::alerts(uid)?).await?;
        let mut answer = alerts(&response, firing_only);
        answer["datasource"] = json!(uid);
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeReader;
    use super::*;

    #[tokio::test]
    async fn dashboards_are_searched_and_detailed() {
        let reader = FakeReader::answering(vec![
            (
                "/api/search",
                Ok(json!([{"uid": "a", "title": "API"}, {"uid": "b", "title": "DB"}])),
            ),
            (
                "/api/dashboards/uid/a",
                Ok(
                    json!({"dashboard": {"uid": "a", "title": "API", "panels": [{"id": 1, "type": "stat", "targets": [{"expr": "up"}]}]}}),
                ),
            ),
        ]);
        let inspect = InspectGrafana::new(reader.clone());
        let found = inspect.dashboards(Some("a"), None, 2).await.unwrap();
        assert_eq!(found["count"], 2);
        assert_eq!(found["more"], true);
        assert_eq!(
            reader.queries()[0][2],
            ("query".to_string(), "a".to_string())
        );
        let detail = inspect.dashboard("a", true).await.unwrap();
        assert_eq!(detail["panels"][0]["queries"], json!(["up"]));
        assert!(inspect.dashboards(None, None, 0).await.is_err());
        assert!(inspect.dashboard("a b", true).await.is_err());
    }

    #[tokio::test]
    async fn alerts_come_from_grafana_or_a_named_alertmanager() {
        let reader = FakeReader::answering(vec![
            (
                "/api/alertmanager/grafana/api/v2/alerts",
                Ok(json!([{"labels": {"alertname": "A"}, "status": {"state": "active"}}])),
            ),
            ("/api/alertmanager/am-1/api/v2/alerts", Ok(json!([]))),
        ]);
        let inspect = InspectGrafana::new(reader);
        let own = inspect.alerts(None, true).await.unwrap();
        assert_eq!(own["count"], 1);
        assert_eq!(own["datasource"], "grafana");
        let external = inspect.alerts(Some("am-1"), false).await.unwrap();
        assert_eq!(external["count"], 0);
        assert_eq!(external["datasource"], "am-1");
        assert!(inspect.alerts(Some("a/b"), true).await.is_err());
    }
}
