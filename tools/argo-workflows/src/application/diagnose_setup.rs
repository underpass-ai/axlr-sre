use std::sync::Arc;

use serde_json::{Value, json};

use crate::domain::{ArgoQuery, ConfigError, ListKind, ServerConfig};
use crate::ports::ArgoReader;

/// `doctor`: the configuration, then one read of the version and one of the
/// namespace, each with the action that fixes it. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    reader: Option<Arc<dyn ArgoReader>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        reader: Option<Arc<dyn ArgoReader>>,
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
                    format!(
                        "server {}, namespace {}",
                        config.server_display(),
                        config.namespace()
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
        let Some(reader) = &self.reader else {
            checks.push(check(
                "reader",
                "failed",
                "no HTTP client",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        match reader.read(&ArgoQuery::Version).await {
            Ok(version) => {
                let shown = version.get("version").cloned().unwrap_or(Value::Null);
                checks.push(check(
                    "server",
                    "ok",
                    format!("Argo Server {shown} answers"),
                    None,
                ));
            }
            Err(error) => {
                checks.push(check(
                    "server",
                    "failed",
                    format!("{error:#}"),
                    Some("check ARGO_SERVER, the network and the TLS trust of this host".into()),
                ));
                return report(checks);
            }
        }
        let probe = ArgoQuery::list(ListKind::Workflows, config.namespace().to_string(), 1, None)
            .expect("a validated namespace and a limit of 1 are a valid query");
        match reader.read(&probe).await {
            Ok(_) => checks.push(check("namespace", "ok", format!("workflows in {} are readable", config.namespace()), None)),
            Err(error) => checks.push(check(
                "namespace",
                "failed",
                format!("{error:#}"),
                Some(format!("give the token's service account `get` and `list` on workflows in {}, or set ARGO_NAMESPACE to a namespace it may read", config.namespace())),
            )),
        }
        report(checks)
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::MissingServer | ConfigError::InvalidServer(_) | ConfigError::InsecureServer => {
            "set ARGO_SERVER to the Argo Server address, https unless it is localhost".into()
        }
        ConfigError::MissingToken => "set ARGO_TOKEN to a service account token that may read the namespace; it is read once at start and never written".into(),
        ConfigError::InvalidNamespace => "set ARGO_NAMESPACE to a Kubernetes namespace name, or unset it for `argo`".into(),
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
    use anyhow::anyhow;
    use async_trait::async_trait;

    use super::*;

    struct Reader {
        version_ok: bool,
        namespace_ok: bool,
    }

    #[async_trait]
    impl ArgoReader for Reader {
        async fn read(&self, query: &ArgoQuery) -> anyhow::Result<Value> {
            match query {
                ArgoQuery::Version if self.version_ok => Ok(json!({"version": "v3.6.0"})),
                ArgoQuery::Version => Err(anyhow!("connection refused")),
                _ if self.namespace_ok => Ok(json!({"items": []})),
                _ => Err(anyhow!("403 Forbidden")),
            }
        }
    }

    fn config() -> Result<ServerConfig, ConfigError> {
        ServerConfig::new(Some("argo.example.com"), Some("ci"), Some("secret-token"))
    }

    #[tokio::test]
    async fn a_working_setup_passes_three_checks_without_the_token() {
        let report = DiagnoseSetup::new(
            config(),
            Some(Arc::new(Reader {
                version_ok: true,
                namespace_ok: true,
            })),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok");
        assert_eq!(report["checks"].as_array().unwrap().len(), 3);
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("v3.6.0")
        );
        assert!(!report.to_string().contains("secret-token"));
    }

    #[tokio::test]
    async fn a_missing_token_stops_at_the_configuration_with_its_action() {
        let config = ServerConfig::new(Some("argo.example.com"), None, None);
        let report = DiagnoseSetup::new(config, None).execute().await;
        assert_eq!(report["status"], "failed");
        assert_eq!(report["checks"].as_array().unwrap().len(), 1);
        assert!(
            report["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("ARGO_TOKEN")
        );
    }

    #[tokio::test]
    async fn every_configuration_error_has_an_action() {
        for error in [
            ConfigError::MissingServer,
            ConfigError::InvalidServer("x".into()),
            ConfigError::InsecureServer,
            ConfigError::MissingToken,
            ConfigError::InvalidNamespace,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }

    #[tokio::test]
    async fn an_unreachable_server_stops_before_the_namespace() {
        let report = DiagnoseSetup::new(
            config(),
            Some(Arc::new(Reader {
                version_ok: false,
                namespace_ok: true,
            })),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "failed");
        assert_eq!(report["checks"].as_array().unwrap().len(), 2);
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("ARGO_SERVER")
        );
    }

    #[tokio::test]
    async fn a_forbidden_namespace_names_the_permission() {
        let report = DiagnoseSetup::new(
            config(),
            Some(Arc::new(Reader {
                version_ok: true,
                namespace_ok: false,
            })),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][2]["action"]
                .as_str()
                .unwrap()
                .contains("ARGO_NAMESPACE")
        );
    }

    #[tokio::test]
    async fn a_missing_reader_is_reported_as_a_bug() {
        let report = DiagnoseSetup::new(config(), None).execute().await;
        assert_eq!(report["checks"][1]["check"], "reader");
    }
}
