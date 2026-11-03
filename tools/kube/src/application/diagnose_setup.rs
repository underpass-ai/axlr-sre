use std::sync::Arc;

use serde_json::{Value, json};

use crate::domain::{ConfigError, Invocation, ServerConfig};
use crate::ports::KubectlRunner;

const NAMESPACES_PROBED: usize = 5;

/// `doctor`: the configuration, the kubectl binary, the API server, then
/// what RBAC lets this identity do in each allowed namespace; with writes,
/// the `diff` program and the patch permission too. Each failure with its
/// action; no credential is shown, kubectl's are never read.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    runner: Option<Arc<dyn KubectlRunner>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        runner: Option<Arc<dyn KubectlRunner>>,
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
                "no kubectl runner",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        match self
            .version(
                runner.as_ref(),
                &Invocation::client_version(),
                "clientVersion",
            )
            .await
        {
            Ok(version) => checks.push(check(
                "kubectl",
                "ok",
                format!("kubectl {version} at {}", config.kubectl()),
                None,
            )),
            Err(error) => {
                checks.push(check("kubectl", "failed", error, Some("set KUBECTL to the kubectl binary, or put kubectl on the PATH of this process".into())));
                return report(checks);
            }
        }
        match self
            .version(runner.as_ref(), &Invocation::version(), "serverVersion")
            .await
        {
            Ok(version) => checks.push(check(
                "server",
                "ok",
                format!("API server {version} answers"),
                None,
            )),
            Err(error) => {
                checks.push(check("server", "failed", error, Some("check the kubeconfig (KUBECONFIG, KUBE_CONTEXT) or the ServiceAccount of this pod, and the network to the API server".into())));
                return report(checks);
            }
        }
        let policy = config.policy();
        if policy.namespaces().is_empty() {
            checks.push(check(
                "namespaces",
                "ok",
                "no allowlist: every call names its namespace and RBAC decides",
                None,
            ));
        }
        for namespace in policy.namespaces().iter().take(NAMESPACES_PROBED) {
            let (status, detail, action) = match self
                .can_i(runner.as_ref(), namespace, "list", "pods")
                .await
            {
                Ok(true) => ("ok", format!("may list pods in {namespace}"), None),
                Ok(false) => (
                    "failed",
                    format!("may not list pods in {namespace}"),
                    Some(format!(
                        "give this identity get, list and watch on pods in {namespace}, or take it out of KUBE_NAMESPACES"
                    )),
                ),
                Err(error) => (
                    "failed",
                    error,
                    Some("check the identity kubectl runs as".into()),
                ),
            };
            checks.push(check(&format!("read {namespace}"), status, detail, action));
        }
        if policy.writes() {
            if runner.program_available("diff") {
                checks.push(check(
                    "diff",
                    "ok",
                    "the diff program kubectl diff runs is on the PATH",
                    None,
                ));
            } else {
                checks.push(check(
                    "diff",
                    "failed",
                    "no diff program on the PATH",
                    Some("install diffutils in the image: kubectl diff shells out to it".into()),
                ));
            }
            for namespace in policy.namespaces().iter().take(NAMESPACES_PROBED) {
                let (status, detail, action) = match self
                    .can_i(runner.as_ref(), namespace, "patch", "deployments")
                    .await
                {
                    Ok(true) => ("ok", format!("may patch deployments in {namespace}"), None),
                    Ok(false) => (
                        "failed",
                        format!("may not patch deployments in {namespace}"),
                        Some(format!(
                            "give this identity patch and update on deployments, statefulsets, hpa and configmaps in {namespace}, or register this server without KUBE_WRITES on the read-only node"
                        )),
                    ),
                    Err(error) => (
                        "failed",
                        error,
                        Some("check the identity kubectl runs as".into()),
                    ),
                };
                checks.push(check(&format!("write {namespace}"), status, detail, action));
            }
        }
        report(checks)
    }

    async fn version(
        &self,
        runner: &dyn KubectlRunner,
        invocation: &Invocation,
        key: &str,
    ) -> Result<String, String> {
        let completed = runner
            .run(invocation)
            .await
            .map_err(|error| format!("{error:#}"))?;
        let parsed: Value = serde_json::from_str(&completed.stdout).unwrap_or(Value::Null);
        match parsed[key]["gitVersion"].as_str() {
            Some(version) => Ok(version.to_string()),
            None if completed.succeeded() => Err(format!("kubectl answered without {key}")),
            None => Err(super::shared::failure(invocation, &completed).to_string()),
        }
    }

    async fn can_i(
        &self,
        runner: &dyn KubectlRunner,
        namespace: &str,
        verb: &str,
        resource: &str,
    ) -> Result<bool, String> {
        let invocation =
            Invocation::can_i(namespace, verb, resource).map_err(|error| error.to_string())?;
        let completed = runner
            .run(&invocation)
            .await
            .map_err(|error| format!("{error:#}"))?;
        match completed.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(super::shared::failure(&invocation, &completed).to_string()),
        }
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::InvalidNamespaces(_) => "set KUBE_NAMESPACES to a comma-separated list of namespace names, or unset it to take the namespace from every call".into(),
        ConfigError::InvalidWrites(_) => "set KUBE_WRITES to true on the node whose ServiceAccount may write, or unset it".into(),
        ConfigError::InvalidMaxReplicas(_) => "set KUBE_MAX_REPLICAS to the most replicas scale may set, 1 to 1000, or unset it for 10".into(),
        ConfigError::InvalidContext(_) => "set KUBE_CONTEXT to a context of the kubeconfig, or unset it for the current one".into(),
        ConfigError::InvalidKubectl => "set KUBECTL to the path of the kubectl binary, or unset it".into(),
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
    use super::super::inspect_cluster::tests::{FakeRunner, exit};
    use super::*;
    use crate::ports::Completed;

    const CLIENT: &str = r#"{"clientVersion":{"gitVersion":"v1.36.2"}}"#;
    const BOTH: &str =
        r#"{"clientVersion":{"gitVersion":"v1.36.2"},"serverVersion":{"gitVersion":"v1.35.0"}}"#;

    fn config(namespaces: &str, writes: &str) -> Result<ServerConfig, ConfigError> {
        ServerConfig::new(Some(namespaces), Some(writes), None, Some("kind-lab"), None)
    }

    fn healthy(answers: Vec<(&'static str, Completed)>) -> Vec<(&'static str, Completed)> {
        let mut all = vec![
            ("version --client", exit(0, CLIENT, "")),
            ("version -o json", exit(0, BOTH, "")),
        ];
        all.extend(answers);
        all
    }

    fn checks(report: &Value) -> Vec<(String, String)> {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["check"].as_str().unwrap().to_string(),
                    c["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_read_only_server_passes_with_its_namespaces_readable() {
        let runner = FakeRunner::answering(healthy(vec![
            ("auth can-i list pods -n prod", exit(0, "yes", "")),
            ("auth can-i list pods -n stage", exit(1, "no", "")),
        ]));
        let report = DiagnoseSetup::new(config("prod,stage", "false"), Some(runner))
            .execute()
            .await;
        assert_eq!(report["status"], "failed");
        assert_eq!(
            checks(&report),
            [
                ("configuration", "ok"),
                ("kubectl", "ok"),
                ("server", "ok"),
                ("read prod", "ok"),
                ("read stage", "failed")
            ]
            .map(|(a, b)| (a.into(), b.into()))
        );
        assert!(
            report["checks"][4]["action"]
                .as_str()
                .unwrap()
                .contains("KUBE_NAMESPACES")
        );
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("v1.36.2")
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("v1.35.0")
        );
    }

    #[tokio::test]
    async fn a_write_server_checks_diff_and_the_patch_permission() {
        let runner = FakeRunner::answering(healthy(vec![
            ("auth can-i list pods", exit(0, "yes", "")),
            ("auth can-i patch deployments", exit(0, "yes", "")),
        ]));
        let report = DiagnoseSetup::new(config("prod", "true"), Some(runner))
            .execute()
            .await;
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(checks(&report).len(), 6);
        assert_eq!(checks(&report)[4].0, "diff");
        assert_eq!(checks(&report)[5].0, "write prod");
        let mut runner = FakeRunner::answering(healthy(vec![
            ("auth can-i list pods", exit(0, "yes", "")),
            ("auth can-i patch deployments", exit(1, "no", "")),
        ]));
        Arc::get_mut(&mut runner).unwrap().diff_available = false;
        let report = DiagnoseSetup::new(config("prod", "true"), Some(runner))
            .execute()
            .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][4]["action"]
                .as_str()
                .unwrap()
                .contains("diffutils")
        );
        assert!(
            report["checks"][5]["action"]
                .as_str()
                .unwrap()
                .contains("KUBE_WRITES")
        );
    }

    #[tokio::test]
    async fn without_an_allowlist_rbac_decides() {
        let runner = FakeRunner::answering(healthy(vec![]));
        let report = DiagnoseSetup::new(config("", "false"), Some(runner))
            .execute()
            .await;
        assert_eq!(report["status"], "ok");
        assert_eq!(checks(&report)[3].0, "namespaces");
    }

    #[tokio::test]
    async fn each_missing_piece_stops_with_its_action() {
        let report = DiagnoseSetup::new(config("Bad", "false"), None)
            .execute()
            .await;
        assert_eq!(
            checks(&report),
            [("configuration".to_string(), "failed".to_string())]
        );
        let report = DiagnoseSetup::new(config("prod", "false"), None)
            .execute()
            .await;
        assert_eq!(checks(&report)[1].0, "runner");
        let mut runner = FakeRunner::answering(vec![]);
        Arc::get_mut(&mut runner).unwrap().fail_to_run = true;
        let report = DiagnoseSetup::new(config("prod", "false"), Some(runner))
            .execute()
            .await;
        assert_eq!(
            checks(&report)[1],
            ("kubectl".to_string(), "failed".to_string())
        );
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("KUBECTL")
        );
        let runner = FakeRunner::answering(vec![
            ("version --client", exit(0, CLIENT, "")),
            (
                "version -o json",
                exit(1, CLIENT, "Unable to connect to the server"),
            ),
        ]);
        let report = DiagnoseSetup::new(config("prod", "false"), Some(runner))
            .execute()
            .await;
        assert_eq!(
            checks(&report)[2],
            ("server".to_string(), "failed".to_string())
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("Unable to connect")
        );
        let runner = FakeRunner::answering(vec![("version --client", exit(0, "{}", ""))]);
        let report = DiagnoseSetup::new(config("prod", "false"), Some(runner))
            .execute()
            .await;
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("without clientVersion")
        );
        let runner = FakeRunner::answering(healthy(vec![(
            "auth can-i",
            exit(2, "", "forbidden to ask"),
        )]));
        let report = DiagnoseSetup::new(config("prod", "false"), Some(runner))
            .execute()
            .await;
        assert_eq!(
            checks(&report)[3],
            ("read prod".to_string(), "failed".to_string())
        );
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("forbidden to ask")
        );
    }

    #[test]
    fn every_configuration_error_has_an_action() {
        for error in [
            ConfigError::InvalidNamespaces("x".into()),
            ConfigError::InvalidWrites("x".into()),
            ConfigError::InvalidMaxReplicas("x".into()),
            ConfigError::InvalidContext("x".into()),
            ConfigError::InvalidKubectl,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }
}
