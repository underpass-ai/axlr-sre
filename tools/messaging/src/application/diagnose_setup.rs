use std::sync::Arc;

use aws_mcp::ports::AwsRunner;
use serde_json::{Value, json};

use crate::domain::{ConfigError, MessagingConfig};

/// `doctor`: the `aws-mcp` checks (configuration, the CLI, the
/// credentials) plus whether writes are on and where a drain writes its
/// evidence. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<MessagingConfig, ConfigError>,
    runner: Option<Arc<dyn AwsRunner>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<MessagingConfig, ConfigError>,
        runner: Option<Arc<dyn AwsRunner>>,
    ) -> Self {
        Self { config, runner }
    }

    pub async fn execute(&self) -> Value {
        let config = match &self.config {
            Ok(config) => config,
            Err(error) => {
                let action = match error {
                    ConfigError::Aws(_) => {
                        "see the aws-mcp variables: AWS_MCP_REGIONS, AWS_MCP_PROFILE, AWS_CLI"
                    }
                    ConfigError::InvalidWrites => {
                        "set MESSAGING_WRITES to true or false, or unset it"
                    }
                    ConfigError::InvalidSnapshotDir => {
                        "set MESSAGING_SNAPSHOT_DIR to a directory this process can write"
                    }
                };
                return json!({"status": "failed", "checks": [{"check": "configuration", "status": "failed", "detail": error.to_string(), "action": action}]});
            }
        };
        let mut report =
            aws_mcp::application::DiagnoseSetup::new(Ok(config.aws().clone()), self.runner.clone())
                .execute()
                .await;
        let checks = report["checks"].as_array_mut();
        if let Some(checks) = checks {
            checks.push(json!({
                "check": "writes",
                "status": "ok",
                "detail": if config.writes() { "send, publish, redrive, redrive_cancel and drain are published; each previews unless confirm=true" } else { "off: only the reads are published" },
                "action": Value::Null,
            }));
            let dir = config.snapshot_dir();
            let writable = dir.exists() && dir.is_dir() || dir.parent().is_some_and(|p| p.exists());
            checks.push(json!({
                "check": "snapshots",
                "status": if writable || !config.writes() { "ok" } else { "failed" },
                "detail": format!("{} ({})", dir.display(), if dir.exists() { "exists" } else { "will be created on the first drain" }),
                "action": if writable || !config.writes() { Value::Null } else { json!("set MESSAGING_SNAPSHOT_DIR to a directory whose parent exists") },
            }));
            let ok = checks.iter().all(|c| c["status"] == "ok");
            report["status"] = json!(if ok { "ok" } else { "failed" });
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeRunner;
    use super::*;
    use aws_mcp::domain::ServerConfig;

    fn config(writes: &str, dir: &str) -> Result<MessagingConfig, ConfigError> {
        MessagingConfig::new(
            ServerConfig::new(Some("eu-west-1"), None, None).unwrap(),
            Some(writes),
            Some(dir),
            None,
        )
    }

    #[tokio::test]
    async fn the_aws_checks_are_followed_by_writes_and_snapshots() {
        let runner = FakeRunner::answering(vec![
            ("--version", Ok(json!("aws-cli/2.36.24"))),
            (
                "sts get-caller-identity",
                Ok(json!({"Account": "123456789012", "Arn": "arn:x"})),
            ),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let report = DiagnoseSetup::new(
            config("true", &dir.path().join("snap").display().to_string()),
            Some(runner.clone()),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok", "{report}");
        let names: Vec<&str> = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["check"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["configuration", "cli", "credentials", "writes", "snapshots"]
        );
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("drain")
        );
        assert!(
            report["checks"][4]["detail"]
                .as_str()
                .unwrap()
                .contains("will be created")
        );
        let nowhere = DiagnoseSetup::new(config("true", "/nope/x/y/snap"), Some(runner.clone()))
            .execute()
            .await;
        assert_eq!(nowhere["status"], "failed");
        assert_eq!(nowhere["checks"][4]["status"], "failed");
        let read_only = DiagnoseSetup::new(config("false", "/nope/x/y/snap"), Some(runner))
            .execute()
            .await;
        assert_eq!(read_only["status"], "ok");
        assert!(
            read_only["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("only the reads")
        );
    }

    #[tokio::test]
    async fn a_bad_configuration_stops_with_its_action() {
        let report = DiagnoseSetup::new(config("maybe", "/x"), None)
            .execute()
            .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("MESSAGING_WRITES")
        );
        let aws = DiagnoseSetup::new(
            Err(ConfigError::Aws(
                aws_mcp::domain::ConfigError::InvalidProfile,
            )),
            None,
        )
        .execute()
        .await;
        assert!(
            aws["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("AWS_MCP_PROFILE")
        );
        let dir = DiagnoseSetup::new(Err(ConfigError::InvalidSnapshotDir), None)
            .execute()
            .await;
        assert!(
            dir["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("MESSAGING_SNAPSHOT_DIR")
        );
        let no_runner = DiagnoseSetup::new(config("true", "/x"), None)
            .execute()
            .await;
        assert_eq!(no_runner["checks"][1]["check"], "runner");
    }
}
