use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog;
use crate::application::{DiagnoseSetup, DiscoverScope, InspectArgo};
use crate::domain::{ArgoQuery, ListKind, ResourceKind};

/// The MCP entry: reads the arguments, builds the validated query and hands
/// it to the use case. A configuration error leaves only `doctor` working.
pub struct ArgoMcpHost {
    inspect: Option<InspectArgo>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    namespace: Option<String>,
}

const INSTRUCTIONS: &str = "Read-only access to Argo Workflows. Call doctor first if anything fails and discover to see the namespace's scope. Listings are paged and summarised; a detail returns the whole resource; logs are bounded to 200 lines and 256 KiB. The namespace defaults to the configured one. Nothing here submits, retries or deletes.";

impl ArgoMcpHost {
    pub fn new(
        inspect: Option<InspectArgo>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
        namespace: Option<String>,
    ) -> Self {
        Self {
            inspect,
            discover,
            doctor,
            namespace,
        }
    }

    fn namespace(&self, args: &Arguments) -> Result<String, ToolOutcome> {
        args.text("namespace")
            .or_else(|| self.namespace.clone())
            .ok_or_else(Self::not_configured)
    }

    fn not_configured() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "the server is not configured",
            "call doctor: it names the variable that is missing",
        )
    }

    fn query(&self, name: &str, args: &Arguments) -> Result<ArgoQuery, ToolOutcome> {
        let namespace = self.namespace(args)?;
        let invalid =
            |error: crate::domain::QueryError| ToolOutcome::failed(error.to_string(), None);
        let bounded = |value: Option<i64>, default: i64| {
            u16::try_from(value.unwrap_or(default)).unwrap_or(u16::MAX)
        };
        match name {
            "list_workflows" | "list_workflow_templates" | "list_cron_workflows" => {
                let kind = match name {
                    "list_workflows" => ListKind::Workflows,
                    "list_workflow_templates" => ListKind::WorkflowTemplates,
                    _ => ListKind::CronWorkflows,
                };
                ArgoQuery::list(
                    kind,
                    namespace,
                    bounded(args.integer("limit"), 25),
                    args.text("continue"),
                )
                .map_err(invalid)
            }
            "get_workflow" | "get_workflow_template" | "get_cron_workflow" => {
                let kind = match name {
                    "get_workflow" => ResourceKind::Workflow,
                    "get_workflow_template" => ResourceKind::WorkflowTemplate,
                    _ => ResourceKind::CronWorkflow,
                };
                let resource = args
                    .text("name")
                    .ok_or_else(|| ToolOutcome::failed("name is required", None))?;
                ArgoQuery::get(kind, namespace, resource).map_err(invalid)
            }
            "get_workflow_logs" => {
                let resource = args
                    .text("name")
                    .ok_or_else(|| ToolOutcome::failed("name is required", None))?;
                ArgoQuery::logs(
                    namespace,
                    resource,
                    args.text("pod_name"),
                    bounded(args.integer("tail_lines"), 100),
                )
                .map_err(invalid)
            }
            _ => Err(ToolOutcome::failed_with_hint(
                format!("unknown tool: {name}"),
                "see tools/list",
            )),
        }
    }
}

#[async_trait]
impl ToolHost for ArgoMcpHost {
    fn name(&self) -> &'static str {
        "argo-workflows-mcp"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn instructions(&self) -> &'static str {
        INSTRUCTIONS
    }

    fn tools(&self) -> Vec<Value> {
        tool_catalog::tools()
    }

    async fn call(&self, name: &str, arguments: Value) -> ToolOutcome {
        let args = Arguments::new(arguments);
        if name == "doctor" {
            return ToolOutcome::Ok(self.doctor.execute().await);
        }
        if name == "discover" {
            let Some(discover) = &self.discover else {
                return Self::not_configured();
            };
            return match discover.execute(args.text("namespace")).await {
                Ok(scope) => ToolOutcome::Ok(scope),
                Err(error) => ToolOutcome::failed(format!("{error:#}"), None),
            };
        }
        let query = match self.query(name, &args) {
            Ok(query) => query,
            Err(outcome) => return outcome,
        };
        let Some(inspect) = &self.inspect else {
            return Self::not_configured();
        };
        match inspect.execute(query).await {
            Ok(value) => ToolOutcome::Ok(value),
            Err(error) => ToolOutcome::failed(format!("{error:#}"), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::domain::ServerConfig;
    use crate::ports::ArgoReader;

    struct FakeReader;

    #[async_trait]
    impl ArgoReader for FakeReader {
        async fn read(&self, query: &ArgoQuery) -> Result<Value> {
            Ok(match query {
                ArgoQuery::Version => json!({"version": "v3.6.0"}),
                ArgoQuery::List { .. } => {
                    json!({"items": [{"metadata": {"name": "run-1"}, "status": {"phase": "Failed"}}]})
                }
                ArgoQuery::Get { name, .. } => {
                    json!({"metadata": {"name": name}, "spec": {"entrypoint": "main"}})
                }
                ArgoQuery::Logs { .. } => json!({"entries": [], "count": 0}),
            })
        }
    }

    fn configured() -> ArgoMcpHost {
        let reader: Arc<dyn ArgoReader> = Arc::new(FakeReader);
        let config = ServerConfig::new(Some("argo.example.com"), Some("ci"), Some("t"));
        ArgoMcpHost::new(
            Some(InspectArgo::new(reader.clone())),
            Some(DiscoverScope::new(reader.clone(), "ci".into())),
            DiagnoseSetup::new(config, Some(reader)),
            Some("ci".into()),
        )
    }

    fn unconfigured() -> ArgoMcpHost {
        ArgoMcpHost::new(
            None,
            None,
            DiagnoseSetup::new(ServerConfig::new(None, None, None), None),
            None,
        )
    }

    fn ok(outcome: ToolOutcome) -> Value {
        match outcome {
            ToolOutcome::Ok(value) => value,
            ToolOutcome::Failed { error, .. } => panic!("failed: {error}"),
        }
    }

    #[tokio::test]
    async fn every_read_reaches_the_reader_with_the_configured_namespace() {
        let host = configured();
        assert_eq!(
            ok(host.call("list_workflows", json!({})).await)["items"][0]["name"],
            "run-1"
        );
        assert_eq!(
            ok(host
                .call("list_workflow_templates", json!({"limit": "5"}))
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("list_cron_workflows", json!({})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("get_workflow", json!({"name": "run-1"})).await)["metadata"]["name"],
            "run-1"
        );
        assert_eq!(
            ok(host
                .call("get_workflow_template", json!({"name": "build"}))
                .await)["metadata"]["name"],
            "build"
        );
        assert_eq!(
            ok(host
                .call("get_cron_workflow", json!({"name": "nightly"}))
                .await)["metadata"]["name"],
            "nightly"
        );
        assert_eq!(
            ok(host
                .call(
                    "get_workflow_logs",
                    json!({"name": "run-1", "tail_lines": 10})
                )
                .await)["count"],
            0
        );
        assert_eq!(
            ok(host.call("discover", json!({})).await)["workflows"]["byPhase"]["Failed"],
            1
        );
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "ok");
    }

    #[tokio::test]
    async fn bad_input_fails_before_the_reader() {
        let host = configured();
        for (name, args) in [
            ("get_workflow", json!({})),
            ("get_workflow", json!({"name": "../bad"})),
            ("get_workflow_logs", json!({})),
            ("get_workflow_logs", json!({"name": "run", "tail_lines": 0})),
            ("list_workflows", json!({"limit": 101})),
            ("list_workflows", json!({"limit": -1})),
            ("list_workflows", json!({"namespace": "Bad"})),
            ("discover", json!({"namespace": "Bad"})),
            ("unknown", json!({})),
        ] {
            assert!(
                matches!(host.call(name, args).await, ToolOutcome::Failed { .. }),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "list_workflows", "get_workflow"] {
            let ToolOutcome::Failed { hint, .. } = host.call(name, json!({"name": "x"})).await
            else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
        let ToolOutcome::Failed { error, .. } =
            host.call("discover", json!({"namespace": "ci"})).await
        else {
            panic!()
        };
        assert!(error.contains("not configured"));
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(configured()));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "argo-workflows-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("Read-only")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 9);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
