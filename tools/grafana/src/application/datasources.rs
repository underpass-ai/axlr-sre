use std::sync::Arc;

use anyhow::Result;

use crate::domain::{Datasource, DatasourceKind, Request, validate_uid};
use crate::ports::GrafanaReader;

/// Which datasource a query goes to: the uid the call names, else the one
/// configured for that kind, else the only (or default) one of that kind in
/// Grafana's listing. The listing is read only when nothing else decides.
pub struct Datasources {
    reader: Arc<dyn GrafanaReader>,
    loki: Option<String>,
    prometheus: Option<String>,
}

impl Datasources {
    pub fn new(
        reader: Arc<dyn GrafanaReader>,
        loki: Option<String>,
        prometheus: Option<String>,
    ) -> Self {
        Self {
            reader,
            loki,
            prometheus,
        }
    }

    pub async fn list(&self) -> Result<Vec<Datasource>> {
        Ok(Datasource::read_all(
            &self.reader.get(&Request::datasources()).await?,
        ))
    }

    pub async fn resolve(&self, kind: &DatasourceKind, requested: Option<&str>) -> Result<String> {
        if let Some(uid) = requested {
            return Ok(validate_uid(uid)?);
        }
        let configured = match kind {
            DatasourceKind::Loki => self.loki.as_deref(),
            DatasourceKind::Prometheus => self.prometheus.as_deref(),
            _ => None,
        };
        if let Some(uid) = configured {
            return Ok(uid.to_string());
        }
        let all = self.list().await?;
        Ok(Datasource::choose(&all, kind)?.uid.clone())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_doubles::FakeReader;
    use super::*;

    #[tokio::test]
    async fn the_call_then_the_configuration_then_the_listing_decide() {
        let reader = FakeReader::answering(vec![(
            "/api/datasources",
            Ok(
                json!([{"uid": "loki-x", "name": "Loki", "type": "loki"}, {"uid": "p", "name": "P", "type": "prometheus"}]),
            ),
        )]);
        let datasources = Datasources::new(reader.clone(), None, Some("prom-env".into()));
        assert_eq!(
            datasources
                .resolve(&DatasourceKind::Loki, Some(" named "))
                .await
                .unwrap(),
            "named"
        );
        assert_eq!(
            datasources
                .resolve(&DatasourceKind::Prometheus, None)
                .await
                .unwrap(),
            "prom-env"
        );
        assert!(reader.paths().is_empty());
        assert_eq!(
            datasources
                .resolve(&DatasourceKind::Loki, None)
                .await
                .unwrap(),
            "loki-x"
        );
        assert_eq!(reader.paths(), ["/api/datasources"]);
        assert!(
            datasources
                .resolve(&DatasourceKind::Loki, Some("bad uid"))
                .await
                .is_err()
        );
        assert!(
            datasources
                .resolve(&DatasourceKind::Alertmanager, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("no Alertmanager datasource")
        );
        assert_eq!(datasources.list().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_denied_listing_is_an_error_only_when_it_is_needed() {
        let reader = FakeReader::answering(vec![(
            "/api/datasources",
            Err("Grafana answered 403".into()),
        )]);
        let datasources = Datasources::new(reader, Some("l".into()), None);
        assert_eq!(
            datasources
                .resolve(&DatasourceKind::Loki, None)
                .await
                .unwrap(),
            "l"
        );
        assert!(
            datasources
                .resolve(&DatasourceKind::Prometheus, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("403")
        );
    }
}
