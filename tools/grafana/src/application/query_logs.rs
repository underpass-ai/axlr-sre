use std::sync::Arc;

use anyhow::Result;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::Datasources;
use crate::domain::{DatasourceKind, Direction, Request, TimeWindow, log_page};
use crate::ports::GrafanaReader;

const DEFAULT_WINDOW_SECONDS: i64 = 3_600;
const VALUES_MAX: usize = 500;

/// Loki through Grafana's proxy: a page of lines, the label names, a label's values.
pub struct QueryLogs {
    reader: Arc<dyn GrafanaReader>,
    datasources: Arc<Datasources>,
}

impl QueryLogs {
    pub fn new(reader: Arc<dyn GrafanaReader>, datasources: Arc<Datasources>) -> Self {
        Self {
            reader,
            datasources,
        }
    }

    pub async fn query(
        &self,
        datasource: Option<&str>,
        query: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: i64,
        direction: Direction,
    ) -> Result<Value> {
        let window = TimeWindow::parse(start, end, Timestamp::now(), DEFAULT_WINDOW_SECONDS)?;
        let uid = self
            .datasources
            .resolve(&DatasourceKind::Loki, datasource)
            .await?;
        let request = Request::loki_query_range(&uid, query, &window, limit, direction)?;
        let mut page = log_page(&self.reader.get(&request).await?, direction, limit)?;
        page["datasource"] = json!(uid);
        page["window"] =
            json!({"start": window.start().to_string(), "end": window.end().to_string()});
        Ok(page)
    }

    pub async fn labels(
        &self,
        datasource: Option<&str>,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Value> {
        let window = TimeWindow::parse(start, end, Timestamp::now(), DEFAULT_WINDOW_SECONDS)?;
        let uid = self
            .datasources
            .resolve(&DatasourceKind::Loki, datasource)
            .await?;
        let response = self
            .reader
            .get(&Request::loki_labels(&uid, &window)?)
            .await?;
        Ok(values_of(&response, &uid))
    }

    pub async fn label_values(
        &self,
        datasource: Option<&str>,
        label: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Value> {
        let window = TimeWindow::parse(start, end, Timestamp::now(), DEFAULT_WINDOW_SECONDS)?;
        let uid = self
            .datasources
            .resolve(&DatasourceKind::Loki, datasource)
            .await?;
        let response = self
            .reader
            .get(&Request::loki_label_values(&uid, label, &window)?)
            .await?;
        let mut answer = values_of(&response, &uid);
        answer["label"] = json!(label.trim());
        Ok(answer)
    }
}

fn values_of(response: &Value, uid: &str) -> Value {
    let all: Vec<&str> = response["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    json!({
        "datasource": uid,
        "count": all.len(),
        "truncated": all.len() > VALUES_MAX,
        "values": all.iter().take(VALUES_MAX).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeReader;
    use super::*;

    const PAGE: &str = r#"{"status":"success","data":{"result":[{"stream":{"app":"web"},"values":[["1793700000000000000","hello"]]}]}}"#;

    fn logs(reader: Arc<FakeReader>) -> QueryLogs {
        let datasources = Arc::new(Datasources::new(
            reader.clone(),
            Some("loki-env".into()),
            None,
        ));
        QueryLogs::new(reader, datasources)
    }

    #[tokio::test]
    async fn a_query_resolves_the_datasource_and_pages() {
        let reader = FakeReader::answering(vec![(
            "/api/datasources/proxy/uid/loki-env/loki/api/v1/query_range",
            Ok(serde_json::from_str(PAGE).unwrap()),
        )]);
        let page = logs(reader.clone())
            .query(
                None,
                "{app=\"web\"}",
                Some("30m"),
                None,
                100,
                Direction::Backward,
            )
            .await
            .unwrap();
        assert_eq!(page["count"], 1);
        assert_eq!(page["datasource"], "loki-env");
        assert_eq!(page["lines"][0]["line"], "hello");
        assert!(page["window"]["start"].as_str().unwrap().ends_with('Z'));
        assert_eq!(
            reader.queries()[0][4],
            ("direction".to_string(), "backward".to_string())
        );
        let named = logs(reader.clone())
            .query(
                Some("other"),
                "{app=\"web\"}",
                None,
                None,
                10,
                Direction::Forward,
            )
            .await;
        assert!(
            named
                .unwrap_err()
                .to_string()
                .contains("unexpected request")
        );
        assert!(
            logs(reader.clone())
                .query(None, "no selector", None, None, 10, Direction::Forward)
                .await
                .is_err()
        );
        assert!(
            logs(reader)
                .query(None, "{a=\"b\"}", Some("bad"), None, 10, Direction::Forward)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn labels_and_values_are_bounded_lists() {
        let many: Vec<String> = (0..600).map(|i| format!("v{i}")).collect();
        let reader = FakeReader::answering(vec![
            (
                "/api/datasources/proxy/uid/loki-env/loki/api/v1/labels",
                Ok(json!({"data": ["app", "namespace"]})),
            ),
            (
                "/api/datasources/proxy/uid/loki-env/loki/api/v1/label/app/values",
                Ok(json!({"data": many})),
            ),
        ]);
        let labels = logs(reader.clone()).labels(None, None, None).await.unwrap();
        assert_eq!(labels["values"], json!(["app", "namespace"]));
        assert_eq!(labels["truncated"], false);
        let values = logs(reader.clone())
            .label_values(None, " app ", Some("1d"), None)
            .await
            .unwrap();
        assert_eq!(values["label"], "app");
        assert_eq!(values["count"], 600);
        assert_eq!(values["truncated"], true);
        assert_eq!(values["values"].as_array().unwrap().len(), 500);
        assert!(
            logs(reader.clone())
                .label_values(None, "1bad", None, None)
                .await
                .is_err()
        );
        assert!(logs(reader).labels(None, Some("8d"), None).await.is_err());
    }
}
