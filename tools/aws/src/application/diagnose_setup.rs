use std::sync::Arc;

use serde_json::{Value, json};

use super::shared::run_json;
use crate::domain::{ConfigError, Invocation, ServerConfig};
use crate::ports::AwsRunner;

/// `doctor`: the configuration, whether the `aws` CLI runs, and who it is
/// in the first region in scope (`sts get-caller-identity`), which is the
/// one call that checks the credentials without reading anything. Without
/// a region allowlist the identity is checked in `us-east-1`, which STS
/// answers for any account. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    runner: Option<Arc<dyn AwsRunner>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        runner: Option<Arc<dyn AwsRunner>>,
    ) -> Self {
        Self { config, runner }
    }

    pub async fn execute(&self) -> Value {
        let mut checks = Vec::new();
        let config = match &self.config {
            Ok(config) => {
                checks.push(check("configuration", "ok", config.summary(), None));
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
        let Some(runner) = &self.runner else {
            checks.push(check(
                "runner",
                "failed",
                "no process runner",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        match runner.run(&Invocation::version()).await {
            Ok(completed) if completed.succeeded() => {
                let version = completed.stdout.trim().to_string() + completed.stderr.trim();
                let v2 = version.starts_with("aws-cli/2");
                checks.push(check(
                    "cli",
                    if v2 { "ok" } else { "failed" },
                    version
                        .lines()
                        .next()
                        .unwrap_or("?")
                        .chars()
                        .take(120)
                        .collect::<String>(),
                    (!v2).then(|| {
                        "install the aws CLI version 2 on this host, or set AWS_CLI to it".into()
                    }),
                ));
                if !v2 {
                    return report(checks);
                }
            }
            Ok(completed) => {
                checks.push(check(
                    "cli",
                    "failed",
                    format!(
                        "aws --version exited {:?}: {}",
                        completed.exit_code,
                        completed
                            .stderr
                            .trim()
                            .chars()
                            .take(200)
                            .collect::<String>()
                    ),
                    Some("install the aws CLI version 2 on this host, or set AWS_CLI to it".into()),
                ));
                return report(checks);
            }
            Err(error) => {
                checks.push(check(
                    "cli",
                    "failed",
                    format!("{error:#}"),
                    Some(format!(
                        "install the aws CLI version 2, or set AWS_CLI to its path (now {})",
                        config.cli().display()
                    )),
                ));
                return report(checks);
            }
        }
        let region = config.regions().default_region().unwrap_or("us-east-1");
        match run_json(runner.as_ref(), &Invocation::caller_identity(region), config.profile()).await {
            Ok(identity) => checks.push(check(
                "credentials",
                "ok",
                format!(
                    "account {}, {}",
                    identity["Account"].as_str().unwrap_or("?"),
                    identity["Arn"].as_str().unwrap_or("?")
                ),
                None,
            )),
            Err(error) => checks.push(check(
                "credentials",
                "failed",
                format!("{error:#}"),
                Some("the hint above says what to do; AWS_MCP_PROFILE picks the profile the CLI uses, unset means the CLI's own chain (a node's role)".into()),
            )),
        }
        if config.regions().allowed().is_empty() {
            checks.push(check(
                "regions",
                "ok",
                "AWS_MCP_REGIONS is not set: every call must name its region; nothing falls back to the CLI's default",
                None,
            ));
        }
        report(checks)
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidRegions(_) => "set AWS_MCP_REGIONS to comma-separated regions such as eu-west-1,us-east-1, or unset it".into(),
        ConfigError::InvalidProfile => "set AWS_MCP_PROFILE to a profile name of the AWS config, or unset it".into(),
        ConfigError::InvalidCli => "set AWS_CLI to the path of the aws executable, or unset it".into(),
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
    use super::super::test_doubles::FakeRunner;
    use super::*;

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

    fn working() -> Vec<(&'static str, Result<Value, String>)> {
        vec![
            (
                "--version",
                Ok(json!("aws-cli/2.36.24 Python/3.14.6 Linux")),
            ),
            (
                "sts get-caller-identity",
                Ok(
                    json!({"Account": "123456789012", "Arn": "arn:aws:sts::123456789012:assumed-role/demo-ro/x"}),
                ),
            ),
        ]
    }

    #[tokio::test]
    async fn a_working_setup_passes_and_names_the_identity() {
        let runner = FakeRunner::answering(working());
        let report = DiagnoseSetup::new(
            ServerConfig::new(Some("eu-west-1"), Some("demo"), None),
            Some(runner.clone()),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(
            statuses(&report),
            [
                ("configuration", "ok"),
                ("cli", "ok"),
                ("credentials", "ok")
            ]
            .map(|(a, b)| (a.into(), b.into()))
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("123456789012")
        );
        assert!(runner.lines()[1].contains("sts get-caller-identity"));
        let any = DiagnoseSetup::new(ServerConfig::new(None, None, None), Some(runner.clone()))
            .execute()
            .await;
        assert_eq!(statuses(&any)[3].0, "regions");
        assert_eq!(any["status"], "ok");
    }

    #[tokio::test]
    async fn a_missing_cli_or_bad_credentials_stop_with_an_action() {
        let runner = FakeRunner::answering(vec![("--version", Ok(json!("aws-cli/1.33.0")))]);
        let report = DiagnoseSetup::new(ServerConfig::new(None, None, None), Some(runner))
            .execute()
            .await;
        assert_eq!(
            statuses(&report),
            [
                ("configuration".to_string(), "ok".to_string()),
                ("cli".into(), "failed".into())
            ]
        );
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("version 2")
        );
        let runner = FakeRunner::answering(vec![("--version", Err("not found".into()))]);
        let report = DiagnoseSetup::new(ServerConfig::new(None, None, None), Some(runner))
            .execute()
            .await;
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("exited")
        );
        let broken = FakeRunner {
            answers: std::sync::Mutex::new(Vec::new()),
            calls: std::sync::Mutex::new(Vec::new()),
            fail_to_run: true,
        };
        let report = DiagnoseSetup::new(
            ServerConfig::new(None, None, Some("/nope/aws")),
            Some(Arc::new(broken)),
        )
        .execute()
        .await;
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("/nope/aws")
        );
        let mut answers = working();
        answers[1] = (
            "sts get-caller-identity",
            Err(
                "Error when retrieving token from sso: Token has expired and refresh failed".into(),
            ),
        );
        let report = DiagnoseSetup::new(
            ServerConfig::new(Some("eu-west-1"), Some("demo"), None),
            Some(FakeRunner::answering(answers)),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("aws sso login --profile demo")
        );
    }

    #[tokio::test]
    async fn each_missing_piece_is_named() {
        let report = DiagnoseSetup::new(ServerConfig::new(Some("x"), None, None), None)
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
                .contains("AWS_MCP_REGIONS")
        );
        let report = DiagnoseSetup::new(ServerConfig::new(None, None, None), None)
            .execute()
            .await;
        assert_eq!(statuses(&report)[1].0, "runner");
        for error in [
            ConfigError::InvalidRegions("x".into()),
            ConfigError::InvalidProfile,
            ConfigError::InvalidCli,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }
}
