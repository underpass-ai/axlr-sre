use std::sync::Arc;

use serde_json::{Value, json};

use crate::domain::{ConfigError, Request, ServerConfig};
use crate::ports::OpsgenieClient;

use super::ReadAlerts;

/// `doctor`: the configuration, the key (the open count answers only with a
/// valid one), then the scope: every team and escalation resolved to an id,
/// or what stops it. No secret is shown. Nothing is written.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    client: Option<Arc<dyn OpsgenieClient>>,
    read: Option<Arc<ReadAlerts>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        client: Option<Arc<dyn OpsgenieClient>>,
        read: Option<Arc<ReadAlerts>>,
    ) -> Self {
        Self {
            config,
            client,
            read,
        }
    }

    pub async fn execute(&self) -> Value {
        let mut checks = Vec::new();
        let config = match &self.config {
            Ok(config) => {
                checks.push(check(
                    "configuration",
                    "ok",
                    format!(
                        "server {}, writes {}, scope {}",
                        config.server_display(),
                        if config.writes() { "on" } else { "off" },
                        config.scope().to_json()
                    ),
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
        let (Some(client), Some(read)) = (&self.client, &self.read) else {
            checks.push(check(
                "client",
                "failed",
                "no HTTP client",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        match client.send(&Request::open_count()).await {
            Ok(count) => checks.push(check(
                "key",
                "ok",
                format!(
                    "the key reads alerts: {} open in the account",
                    count["data"]["count"].as_u64().unwrap_or(0)
                ),
                None,
            )),
            Err(error) => {
                checks.push(check(
                    "key",
                    "failed",
                    format!("{error:#}"),
                    Some(key_action(config).into()),
                ));
                return report(checks);
            }
        }
        match read.resolve_scope().await {
            Ok(scope) => {
                let names = |items: &[crate::domain::Named]| {
                    items
                        .iter()
                        .map(|n| n.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                checks.push(check(
                    "scope",
                    "ok",
                    format!(
                        "teams [{}], escalations [{}]",
                        names(&scope.teams),
                        names(&scope.escalations)
                    ),
                    None,
                ));
            }
            Err(error) => checks.push(check(
                "scope",
                "failed",
                format!("{error:#}"),
                Some("fix OPSGENIE_TEAMS and OPSGENIE_ESCALATIONS: names as Opsgenie spells them, or ids; ids need no listing".into()),
            )),
        }
        if config.writes() {
            checks.push(check(
                "writes",
                "ok",
                "acknowledge, note and close are published; each previews unless confirm=true",
                None,
            ));
        }
        report(checks)
    }
}

fn key_action(config: &ServerConfig) -> &'static str {
    if config.server_display().contains("api.opsgenie.com") {
        "set OPSGENIE_API_KEY to an API integration key with read access; for an EU account set OPSGENIE_URL=https://api.eu.opsgenie.com too"
    } else {
        "set OPSGENIE_API_KEY to an API integration key with read access, and check OPSGENIE_URL"
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidUrl(_) | ConfigError::InsecureUrl => {
            "set OPSGENIE_URL to https://api.opsgenie.com or https://api.eu.opsgenie.com, or unset it"
                .into()
        }
        ConfigError::MissingApiKey => {
            "set OPSGENIE_API_KEY to an API integration key; it is read once at start and never written".into()
        }
        ConfigError::EmptyScope | ConfigError::InvalidScope(_) => {
            "set OPSGENIE_TEAMS and/or OPSGENIE_ESCALATIONS to comma-separated names or ids".into()
        }
        ConfigError::InvalidWrites => "set OPSGENIE_WRITES to true or false, or unset it".into(),
        ConfigError::InvalidUser => "set OPSGENIE_USER to a short name, or unset it".into(),
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
    use super::super::test_doubles::{FakeClient, TEAM_ID, listings};
    use super::*;
    use crate::domain::Scope;

    fn config(teams: &str, writes: Option<&str>) -> Result<ServerConfig, ConfigError> {
        ServerConfig::new(None, Some("secret-key"), Some(teams), None, writes, None)
    }

    fn doctor(config: Result<ServerConfig, ConfigError>, client: Arc<FakeClient>) -> DiagnoseSetup {
        let teams = config
            .as_ref()
            .map(|c| c.scope().teams().join(","))
            .unwrap_or_else(|_| TEAM_ID.into());
        let client: Arc<dyn OpsgenieClient> = client;
        let read = Arc::new(ReadAlerts::new(
            client.clone(),
            Scope::new(Some(&teams), None).unwrap(),
        ));
        DiagnoseSetup::new(config, Some(client), Some(read))
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
    async fn a_working_setup_passes_and_shows_no_key() {
        let mut answers = listings();
        answers.push(("/v2/alerts/count", Ok(json!({"data": {"count": 4}}))));
        let report = doctor(
            config("platform", Some("true")),
            FakeClient::answering(answers),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(
            statuses(&report),
            [
                ("configuration", "ok"),
                ("key", "ok"),
                ("scope", "ok"),
                ("writes", "ok")
            ]
            .map(|(a, b)| (a.into(), b.into()))
        );
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("4 open")
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("Platform")
        );
        assert!(!report.to_string().contains("secret-key"));
    }

    #[tokio::test]
    async fn a_bad_key_names_the_eu_host_and_a_bad_scope_its_variables() {
        let client = FakeClient::answering(vec![(
            "/v2/alerts/count",
            Err("Opsgenie answered 401".into()),
        )]);
        let report = doctor(config("platform", None), client).execute().await;
        assert_eq!(statuses(&report)[1], ("key".into(), "failed".into()));
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("api.eu.opsgenie.com")
        );
        let eu = ServerConfig::new(
            Some("api.eu.opsgenie.com"),
            Some("k"),
            Some("platform"),
            None,
            None,
            None,
        );
        let client = FakeClient::answering(vec![(
            "/v2/alerts/count",
            Err("Opsgenie answered 401".into()),
        )]);
        let report = doctor(eu, client).execute().await;
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("check OPSGENIE_URL")
        );

        let mut answers = listings();
        answers.push(("/v2/alerts/count", Ok(json!({"data": {"count": 0}}))));
        let report = doctor(config("ghost", None), FakeClient::answering(answers))
            .execute()
            .await;
        assert_eq!(report["status"], "failed");
        assert_eq!(statuses(&report)[2], ("scope".into(), "failed".into()));
        assert!(
            report["checks"][2]["action"]
                .as_str()
                .unwrap()
                .contains("OPSGENIE_TEAMS")
        );
    }

    #[tokio::test]
    async fn each_missing_piece_stops_with_its_action() {
        let report = DiagnoseSetup::new(
            ServerConfig::new(None, None, None, None, None, None),
            None,
            None,
        )
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
                .contains("OPSGENIE_API_KEY")
        );
        let report = DiagnoseSetup::new(config("platform", None), None, None)
            .execute()
            .await;
        assert_eq!(statuses(&report)[1].0, "client");
    }

    #[test]
    fn every_configuration_error_has_an_action() {
        for error in [
            ConfigError::InvalidUrl("x".into()),
            ConfigError::InsecureUrl,
            ConfigError::MissingApiKey,
            ConfigError::EmptyScope,
            ConfigError::InvalidScope("OPSGENIE_TEAMS"),
            ConfigError::InvalidWrites,
            ConfigError::InvalidUser,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }
}
