use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::domain::{ConfigError, Datasource, DatasourceKind, Request, ServerConfig};
use crate::ports::GrafanaReader;

/// `doctor`: the configuration, Grafana's health, the token (the current
/// org answers only with a valid one), then the datasources: which Loki and
/// which Prometheus the queries will use, or what decides it. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    reader: Option<Arc<dyn GrafanaReader>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        reader: Option<Arc<dyn GrafanaReader>>,
    ) -> Self {
        Self { config, reader }
    }

    pub async fn execute(&self) -> Value {
        let mut checks = Vec::new();
        let config = match &self.config {
            Ok(config) => {
                checks.push(check(
                    "configuration",
                    "ok",
                    format!("server {}", config.server_display()),
                    None,
                ));
                config
            }
            Err(error) => {
                checks.push(check(
                    "configuration",
                    "failed",
                    error.to_string(),
                    Some(action_for(error)),
                ));
                return report(checks);
            }
        };
        let Some(reader) = &self.reader else {
            checks.push(check(
                "reader",
                "failed",
                "no HTTP client",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        match reader.get(&Request::health()).await {
            Ok(health) => checks.push(check(
                "server",
                "ok",
                format!(
                    "Grafana {} answers, database {}",
                    health["version"].as_str().unwrap_or("?"),
                    health["database"].as_str().unwrap_or("?")
                ),
                None,
            )),
            Err(error) => {
                checks.push(check(
                    "server",
                    "failed",
                    format!("{error:#}"),
                    Some("check GRAFANA_URL, the network and the TLS trust of this host".into()),
                ));
                return report(checks);
            }
        }
        match reader.get(&Request::org()).await {
            Ok(org) => checks.push(check(
                "token",
                "ok",
                format!(
                    "the token reaches organisation {}",
                    org["name"].as_str().unwrap_or("?")
                ),
                None,
            )),
            Err(error) => {
                checks.push(check("token", "failed", format!("{error:#}"), Some("set GRAFANA_TOKEN to a service account token with the Viewer role; it is read once at start and never written".into())));
                return report(checks);
            }
        }
        match reader.get(&Request::datasources()).await {
            Ok(listing) => {
                let all = Datasource::read_all(&listing);
                let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
                for datasource in &all {
                    *by_kind.entry(datasource.kind.as_str()).or_default() += 1;
                }
                checks.push(check(
                    "datasources",
                    "ok",
                    format!(
                        "{} datasources: {}",
                        all.len(),
                        by_kind
                            .iter()
                            .map(|(k, n)| format!("{k} {n}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None,
                ));
                for (kind, configured) in [
                    (DatasourceKind::Loki, config.loki()),
                    (DatasourceKind::Prometheus, config.prometheus()),
                ] {
                    let name = kind.as_str();
                    checks.push(match configured {
                        Some(uid) if all.iter().any(|d| d.uid == uid && d.kind == kind) => {
                            check(name, "ok", format!("{uid}, from {}", kind.variable()), None)
                        }
                        Some(uid) => check(
                            name,
                            "failed",
                            format!("{uid} is not a {name} datasource of this Grafana"),
                            Some(format!(
                                "set {} to one of the uids discover lists, or unset it",
                                kind.variable()
                            )),
                        ),
                        None => match Datasource::choose(&all, &kind) {
                            Ok(chosen) => check(
                                name,
                                "ok",
                                format!(
                                    "{} ({}), the {} one",
                                    chosen.name,
                                    chosen.uid,
                                    if all.iter().filter(|d| d.kind == kind).count() > 1 {
                                        "default"
                                    } else {
                                        "only"
                                    }
                                ),
                                None,
                            ),
                            Err(error) => check(
                                name,
                                "failed",
                                error.to_string(),
                                Some(format!(
                                    "set {} to a uid, or name `datasource` on every call",
                                    kind.variable()
                                )),
                            ),
                        },
                    });
                }
            }
            Err(error) if config.loki().is_some() && config.prometheus().is_some() => {
                checks.push(check("datasources", "ok", format!("the listing is not readable ({error:#}); Loki {} and Prometheus {} come from the configuration", config.loki().unwrap_or_default(), config.prometheus().unwrap_or_default()), None));
            }
            Err(error) => {
                checks.push(check("datasources", "failed", format!("{error:#}"), Some("give the service account the Viewer role, or set GRAFANA_LOKI_DATASOURCE and GRAFANA_PROMETHEUS_DATASOURCE to the uids so the listing is never needed".into())));
            }
        }
        report(checks)
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::MissingUrl | ConfigError::InvalidUrl(_) | ConfigError::InsecureUrl => "set GRAFANA_URL to the Grafana address, https unless it is localhost".into(),
        ConfigError::MissingToken => "set GRAFANA_TOKEN to a service account token with the Viewer role; it is read once at start and never written".into(),
        ConfigError::InvalidDatasource(name) => format!("set {name} to a datasource uid as discover lists them, or unset it"),
    }
}

fn check(name: &str, status: &str, detail: impl Into<String>, action: Option<String>) -> Value {
    json!({"check": name, "status": status, "detail": detail.into(), "action": action})
}

fn report(checks: Vec<Value>) -> Value {
    let ok = checks.iter().all(|c| c["status"] == "ok");
    json!({"status": if ok { "ok" } else { "failed" }, "checks": checks})
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeReader;
    use super::*;

    fn config(loki: Option<&str>, prometheus: Option<&str>) -> Result<ServerConfig, ConfigError> {
        ServerConfig::new(
            Some("grafana.example.com"),
            Some("secret-token"),
            loki,
            prometheus,
        )
    }

    fn healthy() -> Vec<(&'static str, Result<Value, String>)> {
        vec![
            (
                "/api/health",
                Ok(json!({"version": "12.1.0", "database": "ok"})),
            ),
            ("/api/org", Ok(json!({"name": "Main Org."}))),
            (
                "/api/datasources",
                Ok(
                    json!([{"uid": "loki-1", "name": "Loki", "type": "loki"}, {"uid": "p1", "name": "P1", "type": "prometheus"}, {"uid": "p2", "name": "P2", "type": "prometheus"}]),
                ),
            ),
        ]
    }

    fn statuses(report: &Value) -> Vec<(String, String)> {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["check"].as_str().unwrap().into(),
                    c["status"].as_str().unwrap().into(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_healthy_grafana_passes_and_an_ambiguous_prometheus_fails_with_its_action() {
        let report = DiagnoseSetup::new(config(None, None), Some(FakeReader::answering(healthy())))
            .execute()
            .await;
        assert_eq!(report["status"], "failed");
        assert_eq!(
            statuses(&report),
            [
                ("configuration", "ok"),
                ("server", "ok"),
                ("token", "ok"),
                ("datasources", "ok"),
                ("loki", "ok"),
                ("prometheus", "failed")
            ]
            .map(|(a, b)| (a.into(), b.into()))
        );
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("12.1.0")
        );
        assert!(
            report["checks"][4]["detail"]
                .as_str()
                .unwrap()
                .contains("the only one")
        );
        assert!(
            report["checks"][5]["action"]
                .as_str()
                .unwrap()
                .contains("GRAFANA_PROMETHEUS_DATASOURCE")
        );
        assert!(!report.to_string().contains("secret-token"));
    }

    #[tokio::test]
    async fn configured_uids_are_checked_against_the_listing() {
        let report = DiagnoseSetup::new(
            config(Some("loki-1"), Some("p9")),
            Some(FakeReader::answering(healthy())),
        )
        .execute()
        .await;
        assert_eq!(statuses(&report)[4], ("loki".into(), "ok".into()));
        assert_eq!(statuses(&report)[5], ("prometheus".into(), "failed".into()));
        assert!(
            report["checks"][5]["detail"]
                .as_str()
                .unwrap()
                .contains("p9 is not a prometheus datasource")
        );
    }

    #[tokio::test]
    async fn a_denied_listing_is_fine_only_when_both_uids_are_configured() {
        let mut answers = healthy();
        answers[2] = ("/api/datasources", Err("Grafana answered 403".into()));
        let report = DiagnoseSetup::new(
            config(Some("l"), Some("p")),
            Some(FakeReader::answering(answers.clone())),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok");
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("come from the configuration")
        );
        let report = DiagnoseSetup::new(
            config(Some("l"), None),
            Some(FakeReader::answering(answers)),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][3]["action"]
                .as_str()
                .unwrap()
                .contains("Viewer")
        );
    }

    #[tokio::test]
    async fn each_missing_piece_stops_with_its_action() {
        let report = DiagnoseSetup::new(ServerConfig::new(None, None, None, None), None)
            .execute()
            .await;
        assert_eq!(
            statuses(&report),
            [("configuration".to_string(), "failed".to_string())]
        );
        assert!(
            report["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("GRAFANA_URL")
        );
        let report = DiagnoseSetup::new(config(None, None), None).execute().await;
        assert_eq!(statuses(&report)[1].0, "reader");
        let report = DiagnoseSetup::new(
            config(None, None),
            Some(FakeReader::answering(vec![(
                "/api/health",
                Err("connection refused".into()),
            )])),
        )
        .execute()
        .await;
        assert_eq!(statuses(&report)[1], ("server".into(), "failed".into()));
        let report = DiagnoseSetup::new(
            config(None, None),
            Some(FakeReader::answering(vec![
                ("/api/health", Ok(json!({}))),
                ("/api/org", Err("Grafana answered 401".into())),
            ])),
        )
        .execute()
        .await;
        assert_eq!(statuses(&report)[2], ("token".into(), "failed".into()));
        assert!(
            report["checks"][2]["action"]
                .as_str()
                .unwrap()
                .contains("GRAFANA_TOKEN")
        );
    }

    #[test]
    fn every_configuration_error_has_an_action() {
        for error in [
            ConfigError::MissingUrl,
            ConfigError::InvalidUrl("x".into()),
            ConfigError::InsecureUrl,
            ConfigError::MissingToken,
            ConfigError::InvalidDatasource("GRAFANA_LOKI_DATASOURCE"),
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }
}
