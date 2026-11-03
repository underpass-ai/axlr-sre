use std::sync::Arc;

use anyhow::Result;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::Datasources;
use crate::domain::{DatasourceKind, Request, TimeWindow, instant, series};
use crate::ports::GrafanaReader;

const DEFAULT_WINDOW_SECONDS: i64 = 3_600;

/// Prometheus through Grafana's proxy: an instant query, a range query.
pub struct QueryMetrics {
    reader: Arc<dyn GrafanaReader>,
    datasources: Arc<Datasources>,
}

impl QueryMetrics {
    pub fn new(reader: Arc<dyn GrafanaReader>, datasources: Arc<Datasources>) -> Self {
        Self {
            reader,
            datasources,
        }
    }

    pub async fn instant(
        &self,
        datasource: Option<&str>,
        query: &str,
        time: Option<&str>,
    ) -> Result<Value> {
        let now = Timestamp::now();
        let at = match time {
            Some(text) => instant(text, now)?,
            None => now,
        };
        let uid = self
            .datasources
            .resolve(&DatasourceKind::Prometheus, datasource)
            .await?;
        let response = self
            .reader
            .get(&Request::prometheus_query(&uid, query, at)?)
            .await?;
        let mut answer = series(&response)?;
        answer["datasource"] = json!(uid);
        answer["at"] = json!(at.to_string());
        Ok(answer)
    }

    pub async fn range(
        &self,
        datasource: Option<&str>,
        query: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Value> {
        let window = TimeWindow::parse(start, end, Timestamp::now(), DEFAULT_WINDOW_SECONDS)?;
        let uid = self
            .datasources
            .resolve(&DatasourceKind::Prometheus, datasource)
            .await?;
        let response = self
            .reader
            .get(&Request::prometheus_query_range(&uid, query, &window)?)
            .await?;
        let mut answer = series(&response)?;
        answer["datasource"] = json!(uid);
        answer["window"] = json!({"start": window.start().to_string(), "end": window.end().to_string(), "step": window.step_seconds()});
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeReader;
    use super::*;

    fn metrics(reader: Arc<FakeReader>) -> QueryMetrics {
        let datasources = Arc::new(Datasources::new(reader.clone(), None, None));
        QueryMetrics::new(reader, datasources)
    }

    #[tokio::test]
    async fn instant_and_range_resolve_the_datasource_from_the_listing() {
        let reader = FakeReader::answering(vec![
            (
                "/api/datasources/proxy/uid/prom/api/v1/query_range",
                Ok(
                    json!({"data": {"resultType": "matrix", "result": [{"metric": {}, "values": [[1, "1"]]}]}}),
                ),
            ),
            (
                "/api/datasources/proxy/uid/prom/api/v1/query",
                Ok(
                    json!({"data": {"resultType": "vector", "result": [{"metric": {"job": "a"}, "value": [1, "1"]}]}}),
                ),
            ),
            (
                "/api/datasources",
                Ok(json!([{"uid": "prom", "name": "P", "type": "prometheus"}])),
            ),
        ]);
        let up = metrics(reader.clone())
            .instant(None, "up", Some("5m"))
            .await
            .unwrap();
        assert_eq!(up["type"], "vector");
        assert_eq!(up["datasource"], "prom");
        assert!(up["at"].as_str().unwrap().ends_with('Z'));
        let rate = metrics(reader.clone())
            .range(Some("prom"), "rate(x[5m])", Some("2h"), None)
            .await
            .unwrap();
        assert_eq!(rate["type"], "matrix");
        assert_eq!(rate["window"]["step"], 72);
        assert_eq!(
            reader
                .paths()
                .iter()
                .filter(|p| *p == "/api/datasources")
                .count(),
            1
        );
        assert!(
            metrics(reader.clone())
                .instant(None, "", None)
                .await
                .is_err()
        );
        assert!(
            metrics(reader.clone())
                .instant(None, "up", Some("soon"))
                .await
                .is_err()
        );
        assert!(
            metrics(reader)
                .range(None, "up", Some("1m"), Some("2m"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_rejected_query_says_why() {
        let reader = FakeReader::answering(vec![(
            "/api/datasources/proxy/uid/prom/api/v1/query",
            Ok(json!({"status": "error", "error": "parse error"})),
        )]);
        let error = metrics(reader)
            .instant(Some("prom"), "up(", None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("parse error"));
    }
}
