use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog::{self, WRITE_TOOLS};
use crate::application::{ChangeCluster, DiagnoseSetup, DiscoverScope, InspectCluster};
use crate::domain::{Policy, RolloutAction, RolloutTarget, ScaleTarget};

/// The MCP entry: reads the arguments, resolves the namespace against the
/// policy and hands the call to a use case. A configuration error leaves
/// only `doctor` working; a read-only server refuses the writes by name.
pub struct KubeMcpHost {
    inspect: Option<InspectCluster>,
    change: Option<ChangeCluster>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    policy: Option<Policy>,
}

const INSTRUCTIONS: &str = "Kubernetes through kubectl, one tool per verb. Call doctor first if anything fails and discover to see what is in scope. Every call names its namespace, checked against the server's allowlist; secrets are never read. Answers are bounded: summaries, pages, tails. A write (diff, apply, rollout_restart, rollout_undo, scale) exists only on a server registered with writes, previews unless confirm is true, and never touches a Secret, Namespace, PVC, CRD, Node, RBAC, NetworkPolicy or admission webhook.";

impl KubeMcpHost {
    pub fn new(
        inspect: Option<InspectCluster>,
        change: Option<ChangeCluster>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
        policy: Option<Policy>,
    ) -> Self {
        Self {
            inspect,
            change,
            discover,
            doctor,
            policy,
        }
    }

    fn not_configured() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "the server is not configured",
            "call doctor: it names the variable that is wrong",
        )
    }

    fn read_only() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "this server is read-only",
            "register kube-mcp with KUBE_WRITES=true on the node whose ServiceAccount may write",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(policy), Some(inspect), Some(discover)) =
            (&self.policy, &self.inspect, &self.discover)
        else {
            return Self::not_configured();
        };
        if WRITE_TOOLS.contains(&name) && !policy.writes() {
            return Self::read_only();
        }
        let namespace = match policy.namespace(args.text("namespace").as_deref()) {
            Ok(namespace) => namespace,
            Err(error) => return ToolOutcome::failed(error.to_string(), None),
        };
        let text = |key: &str| args.text(key);
        let required = |key: &str| {
            args.text(key)
                .ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let rollout_kind = || {
            RolloutTarget::parse(&args.text("kind").unwrap_or_else(|| "deployment".into()))
                .map_err(|error| ToolOutcome::failed(error.to_string(), None))
        };
        let result = match name {
            "discover" => discover.execute(text("namespace").as_deref()).await,
            "get" => {
                let kind = match required("kind") {
                    Ok(kind) => kind,
                    Err(outcome) => return outcome,
                };
                inspect
                    .get(
                        &namespace,
                        &kind,
                        text("name").as_deref(),
                        text("selector").as_deref(),
                        args.integer("limit").unwrap_or(50),
                        args.flag("raw", false),
                    )
                    .await
            }
            "describe" => match (required("kind"), required("name")) {
                (Ok(kind), Ok(name)) => inspect.describe(&namespace, &kind, &name).await,
                (Err(outcome), _) | (_, Err(outcome)) => return outcome,
            },
            "logs" => {
                let pod = match required("pod") {
                    Ok(pod) => pod,
                    Err(outcome) => return outcome,
                };
                inspect
                    .logs(
                        &namespace,
                        &pod,
                        text("container").as_deref(),
                        args.integer("tail").unwrap_or(200),
                        text("since").as_deref(),
                        args.flag("previous", false),
                    )
                    .await
            }
            "events" => {
                inspect
                    .events(
                        &namespace,
                        text("object").as_deref(),
                        args.flag("warnings_only", true),
                        args.integer("limit").unwrap_or(50),
                    )
                    .await
            }
            "rollout_status" | "rollout_history" => match (rollout_kind(), required("name")) {
                (Ok(kind), Ok(target)) if name == "rollout_status" => {
                    inspect
                        .rollout_status(
                            &namespace,
                            kind,
                            &target,
                            args.integer("timeout_seconds").unwrap_or(120),
                        )
                        .await
                }
                (Ok(kind), Ok(target)) => inspect.rollout_history(&namespace, kind, &target).await,
                (Err(outcome), _) | (_, Err(outcome)) => return outcome,
            },
            "diff" | "apply" | "rollout_restart" | "rollout_undo" | "scale" => {
                let Some(change) = &self.change else {
                    return Self::read_only();
                };
                let confirm = args.flag("confirm", false);
                match name {
                    "diff" => match required("manifest") {
                        Ok(manifest) => change.diff(&namespace, &manifest).await,
                        Err(outcome) => return outcome,
                    },
                    "apply" => match required("manifest") {
                        Ok(manifest) => change.apply(&namespace, &manifest, confirm).await,
                        Err(outcome) => return outcome,
                    },
                    "rollout_restart" | "rollout_undo" => {
                        match (rollout_kind(), required("name")) {
                            (Ok(kind), Ok(target)) => {
                                let action = if name == "rollout_restart" {
                                    RolloutAction::Restart
                                } else {
                                    RolloutAction::Undo
                                };
                                let revision = args.integer("revision").filter(|r| *r != 0);
                                change
                                    .rollout(&namespace, action, kind, &target, revision, confirm)
                                    .await
                            }
                            (Err(outcome), _) | (_, Err(outcome)) => return outcome,
                        }
                    }
                    _ => {
                        let kind = ScaleTarget::parse(
                            &args.text("kind").unwrap_or_else(|| "deployment".into()),
                        );
                        match (kind, required("name"), args.integer("replicas")) {
                            (Ok(kind), Ok(target), Some(replicas)) => {
                                change
                                    .scale(&namespace, kind, &target, replicas, confirm)
                                    .await
                            }
                            (Err(error), _, _) => {
                                return ToolOutcome::failed(error.to_string(), None);
                            }
                            (_, Err(outcome), _) => return outcome,
                            (_, _, None) => {
                                return ToolOutcome::failed("replicas is required", None);
                            }
                        }
                    }
                }
            }
            _ => {
                return ToolOutcome::failed_with_hint(
                    format!("unknown tool: {name}"),
                    "see tools/list",
                );
            }
        };
        match result {
            Ok(value) => ToolOutcome::Ok(value),
            Err(error) => ToolOutcome::failed(format!("{error:#}"), None),
        }
    }
}

#[async_trait]
impl ToolHost for KubeMcpHost {
    fn name(&self) -> &'static str {
        "kube-mcp"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn instructions(&self) -> &'static str {
        INSTRUCTIONS
    }

    fn tools(&self) -> Vec<Value> {
        tool_catalog::tools(self.policy.as_ref().is_some_and(Policy::writes))
    }

    async fn call(&self, name: &str, arguments: Value) -> ToolOutcome {
        let args = Arguments::new(arguments);
        if name == "doctor" {
            return ToolOutcome::Ok(self.doctor.execute().await);
        }
        self.dispatch(name, &args).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::application::inspect_cluster::tests::{FakeRunner, exit};
    use crate::domain::ServerConfig;
    use crate::ports::{Completed, KubectlRunner};

    const DEPLOY: &str = r#"{"kind":"Deployment","metadata":{"name":"web","generation":1},"spec":{"replicas":2},"status":{"readyReplicas":2}}"#;
    const LIST: &str = r#"{"kind":"List","items":[{"kind":"Pod","metadata":{"name":"a"},"status":{"phase":"Running"}}]}"#;

    fn answers() -> Vec<(&'static str, Completed)> {
        vec![
            (
                "version",
                exit(
                    0,
                    r#"{"clientVersion":{"gitVersion":"v1"},"serverVersion":{"gitVersion":"v1"}}"#,
                    "",
                ),
            ),
            ("auth can-i", exit(0, "yes", "")),
            ("get deployment web", exit(0, DEPLOY, "")),
            ("get deployments", exit(0, LIST, "")),
            ("get pods", exit(0, LIST, "")),
            ("get events", exit(0, LIST, "")),
            ("describe", exit(0, "Name: a", "")),
            ("logs", exit(0, "l1\n", "")),
            ("rollout status", exit(0, "rolled out", "")),
            ("rollout history", exit(0, "REVISION", "")),
            ("rollout restart", exit(0, "restarted", "")),
            ("rollout undo", exit(0, "rolled back", "")),
            ("diff", exit(1, "+x", "")),
            ("apply", exit(0, "deployment.apps/web", "")),
            ("scale", exit(0, "scaled", "")),
        ]
    }

    fn host(writes: bool) -> KubeMcpHost {
        let runner: Arc<dyn KubectlRunner> = FakeRunner::answering(answers());
        let config = ServerConfig::new(
            Some("prod,stage"),
            Some(if writes { "true" } else { "false" }),
            None,
            None,
            None,
        );
        let policy = config.as_ref().unwrap().policy().clone();
        KubeMcpHost::new(
            Some(InspectCluster::new(runner.clone())),
            writes.then(|| ChangeCluster::new(runner.clone(), policy.clone())),
            Some(DiscoverScope::new(runner.clone(), policy.clone())),
            DiagnoseSetup::new(config, Some(runner)),
            Some(policy),
        )
    }

    fn unconfigured() -> KubeMcpHost {
        let config = ServerConfig::new(Some("Bad"), None, None, None, None);
        KubeMcpHost::new(None, None, None, DiagnoseSetup::new(config, None), None)
    }

    fn ok(outcome: ToolOutcome) -> Value {
        match outcome {
            ToolOutcome::Ok(value) => value,
            ToolOutcome::Failed { error, .. } => panic!("failed: {error}"),
        }
    }

    fn failed(outcome: ToolOutcome) -> (String, Option<String>) {
        match outcome {
            ToolOutcome::Failed { error, hint } => (error, hint),
            ToolOutcome::Ok(value) => panic!("succeeded: {value}"),
        }
    }

    #[tokio::test]
    async fn every_read_answers_with_the_default_namespace() {
        let host = host(false);
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "ok");
        assert_eq!(
            ok(host.call("discover", json!({"namespace": "stage"})).await)["namespaces"][0]["namespace"],
            "stage"
        );
        let pods = ok(host
            .call("get", json!({"kind": "pods", "limit": "10"}))
            .await);
        assert_eq!(pods["namespace"], "prod");
        assert_eq!(pods["count"], 1);
        assert_eq!(
            ok(host
                .call("describe", json!({"kind": "pod", "name": "a"}))
                .await)["text"],
            "Name: a"
        );
        assert_eq!(
            ok(host
                .call("logs", json!({"pod": "a", "previous": true}))
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("events", json!({"object": "a"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("rollout_status", json!({"name": "web"})).await)["status"],
            "complete"
        );
        assert_eq!(
            ok(host
                .call(
                    "rollout_history",
                    json!({"name": "web", "kind": "statefulset"})
                )
                .await)["text"],
            "REVISION"
        );
    }

    #[tokio::test]
    async fn missing_and_bad_arguments_fail_before_kubectl() {
        let host = host(true);
        for (name, args, expected) in [
            ("get", json!({}), "kind is required"),
            ("get", json!({"kind": "secrets"}), "secrets"),
            (
                "get",
                json!({"kind": "pods", "namespace": "other"}),
                "allowlist",
            ),
            ("describe", json!({"kind": "pod"}), "name is required"),
            ("describe", json!({"name": "a"}), "kind is required"),
            ("logs", json!({}), "pod is required"),
            ("logs", json!({"pod": "a", "tail": 0}), "tail"),
            ("rollout_status", json!({}), "name is required"),
            (
                "rollout_status",
                json!({"name": "web", "kind": "job"}),
                "rollout kind",
            ),
            (
                "rollout_history",
                json!({"name": "web", "kind": "job"}),
                "rollout kind",
            ),
            (
                "rollout_restart",
                json!({"kind": "job", "name": "web"}),
                "rollout kind",
            ),
            ("rollout_undo", json!({}), "name is required"),
            ("diff", json!({}), "manifest is required"),
            ("apply", json!({}), "manifest is required"),
            (
                "apply",
                json!({"manifest": "kind: Secret\nmetadata: {name: s}"}),
                "never written",
            ),
            ("scale", json!({"name": "web"}), "replicas is required"),
            ("scale", json!({"replicas": 1}), "name is required"),
            (
                "scale",
                json!({"name": "web", "replicas": 1, "kind": "daemonset"}),
                "scale kind",
            ),
            ("scale", json!({"name": "web", "replicas": 99}), "replicas"),
            ("unknown", json!({}), "unknown tool"),
        ] {
            let (error, _) = failed(host.call(name, args).await);
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn writes_preview_and_run_on_confirm() {
        let host = host(true);
        assert_eq!(
            ok(host
                .call(
                    "diff",
                    json!({"manifest": "kind: ConfigMap\nmetadata: {name: c}"})
                )
                .await)["differences"],
            true
        );
        let preview = ok(host
            .call(
                "apply",
                json!({"manifest": "kind: ConfigMap\nmetadata: {name: c}"}),
            )
            .await);
        assert_eq!(preview["mode"], "preview");
        let applied = ok(host
            .call(
                "apply",
                json!({"manifest": "kind: ConfigMap\nmetadata: {name: c}", "confirm": true}),
            )
            .await);
        assert_eq!(applied["mode"], "applied");
        assert_eq!(
            ok(host.call("rollout_restart", json!({"name": "web"})).await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call("rollout_restart", json!({"name": "web", "confirm": true}))
                .await)["mode"],
            "done"
        );
        assert_eq!(
            ok(host
                .call(
                    "rollout_undo",
                    json!({"name": "web", "revision": 2, "confirm": true})
                )
                .await)["mode"],
            "done"
        );
        assert_eq!(
            ok(host
                .call("rollout_undo", json!({"name": "web", "revision": 0}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call("scale", json!({"name": "web", "replicas": 3}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call(
                    "scale",
                    json!({"name": "web", "replicas": 3, "confirm": true})
                )
                .await)["mode"],
            "done"
        );
        assert_eq!(host.tools().len(), 13);
    }

    #[tokio::test]
    async fn a_read_only_server_refuses_every_write_by_name() {
        let host = host(false);
        assert_eq!(host.tools().len(), 8);
        for name in WRITE_TOOLS {
            let (error, hint) = failed(
                host.call(name, json!({"name": "web", "manifest": "x", "replicas": 1}))
                    .await,
            );
            assert!(error.contains("read-only"), "{name}");
            assert!(hint.unwrap().contains("KUBE_WRITES"));
        }
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        assert_eq!(host.tools().len(), 8);
        for name in ["discover", "get", "apply"] {
            let (error, hint) = failed(host.call(name, json!({"kind": "pods"})).await);
            assert!(error.contains("not configured"), "{name}");
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host(true)));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "kube-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("one tool per verb")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 13);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
