use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::Datasources;
use crate::domain::{Datasource, Request, alerts, dashboard_summaries};
use crate::ports::GrafanaReader;

const DATASOURCES_MAX: usize = 50;
const DASHBOARDS_SHOWN: i64 = 20;

/// `discover`: the datasources with their uids, the first dashboards and
/// how many alerts are in each state. A part that cannot be read says so
/// instead of failing the rest.
pub struct DiscoverScope {
    reader: Arc<dyn GrafanaReader>,
    datasources: Arc<Datasources>,
}

impl DiscoverScope {
    pub fn new(reader: Arc<dyn GrafanaReader>, datasources: Arc<Datasources>) -> Self {
        Self {
            reader,
            datasources,
        }
    }

    pub async fn execute(&self) -> Result<Value> {
        let datasources = match self.datasources.list().await {
            Ok(all) => json!({
                "count": all.len(),
                "truncated": all.len() > DATASOURCES_MAX,
                "items": all.iter().take(DATASOURCES_MAX).map(Datasource::to_json).collect::<Vec<_>>(),
            }),
            Err(error) => json!({"unavailable": format!("{error:#}")}),
        };
        let dashboards = match self
            .reader
            .get(&Request::search_dashboards(None, None, DASHBOARDS_SHOWN)?)
            .await
        {
            Ok(response) => {
                let items = dashboard_summaries(&response);
                json!({"shown": items.len(), "more": i64::try_from(items.len()).unwrap_or(i64::MAX) >= DASHBOARDS_SHOWN, "items": items})
            }
            Err(error) => json!({"unavailable": format!("{error:#}")}),
        };
        let firing = match self.reader.get(&Request::alerts("grafana")?).await {
            Ok(response) => alerts(&response, false)["byState"].clone(),
            Err(error) => json!({"unavailable": format!("{error:#}")}),
        };
        Ok(json!({"datasources": datasources, "dashboards": dashboards, "alertsByState": firing}))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeReader;
    use super::*;

    fn discover(reader: Arc<FakeReader>) -> DiscoverScope {
        let datasources = Arc::new(Datasources::new(reader.clone(), None, None));
        DiscoverScope::new(reader, datasources)
    }

    #[tokio::test]
    async fn everything_readable_is_listed() {
        let reader = FakeReader::answering(vec![
            (
                "/api/datasources",
                Ok(json!([{"uid": "l", "name": "Loki", "type": "loki"}])),
            ),
            ("/api/search", Ok(json!([{"uid": "a", "title": "API"}]))),
            (
                "/api/alertmanager/grafana",
                Ok(json!([{"status": {"state": "active"}}, {"status": {"state": "active"}}])),
            ),
        ]);
        let scope = discover(reader).execute().await.unwrap();
        assert_eq!(scope["datasources"]["count"], 1);
        assert_eq!(scope["datasources"]["items"][0]["uid"], "l");
        assert_eq!(scope["dashboards"]["shown"], 1);
        assert_eq!(scope["dashboards"]["more"], false);
        assert_eq!(scope["alertsByState"]["active"], 2);
    }

    #[tokio::test]
    async fn an_unreadable_part_says_so() {
        let reader = FakeReader::answering(vec![("/api/search", Ok(json!([])))]);
        let scope = discover(reader).execute().await.unwrap();
        assert!(
            scope["datasources"]["unavailable"]
                .as_str()
                .unwrap()
                .contains("unexpected")
        );
        assert!(scope["alertsByState"]["unavailable"].is_string());
        assert_eq!(scope["dashboards"]["shown"], 0);
    }
}
