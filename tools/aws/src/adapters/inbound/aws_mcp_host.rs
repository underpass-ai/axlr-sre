use async_trait::async_trait;
use serde_json::{Map, Value};
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog;
use crate::application::{DiagnoseSetup, DiscoverScope, InspectLambda, QueryLogs, QueryMetrics};

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working.
pub struct AwsMcpHost {
    metrics: Option<QueryMetrics>,
    lambda: Option<InspectLambda>,
    logs: Option<QueryLogs>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
}

const INSTRUCTIONS: &str = "Read-only access to AWS through the aws CLI: CloudWatch metrics and alarms, Lambda functions with their aliases, triggers and log groups, and CloudWatch Logs. Every call names its region or takes the first of AWS_MCP_REGIONS; nothing falls back to the CLI's default region. Call doctor first if anything fails and discover to see who the CLI is and what each region holds. Windows take RFC 3339 or how long ago (15m, 3h, 1d), fourteen days at most; series are compacted to the points where the value changes; `no data` means CloudWatch has not one point, which is not a failure. Nothing here writes.";

impl AwsMcpHost {
    pub fn new(
        metrics: Option<QueryMetrics>,
        lambda: Option<InspectLambda>,
        logs: Option<QueryLogs>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
    ) -> Self {
        Self {
            metrics,
            lambda,
            logs,
            discover,
            doctor,
        }
    }

    fn not_configured() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "the server is not configured",
            "call doctor: it names the variable that is wrong",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(metrics), Some(lambda), Some(logs), Some(discover)) =
            (&self.metrics, &self.lambda, &self.logs, &self.discover)
        else {
            return Self::not_configured();
        };
        let text = |key: &str| args.text(key).filter(|t| !t.trim().is_empty());
        let required = |key: &str| {
            text(key).ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let region = text("region");
        let result = match name {
            "discover" => discover.execute(region.as_deref()).await,
            "metric_query" => match (required("namespace"), required("metric")) {
                (Ok(namespace), Ok(metric)) => {
                    let dimensions: Map<String, Value> = args
                        .get("dimensions")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    metrics
                        .metric_query(
                            region.as_deref(),
                            &namespace,
                            &metric,
                            &dimensions,
                            &text("stat").unwrap_or_else(|| "Sum".into()),
                            args.integer("period").unwrap_or(300),
                            text("start").as_deref(),
                            text("end").as_deref(),
                        )
                        .await
                }
                (Err(outcome), _) | (_, Err(outcome)) => return outcome,
            },
            "lambda_health" => match required("function") {
                Ok(function) => {
                    metrics
                        .lambda_health(
                            region.as_deref(),
                            &function,
                            args.integer("period").unwrap_or(300),
                            text("start").as_deref(),
                            text("end").as_deref(),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "alarms" => {
                metrics
                    .alarms(
                        region.as_deref(),
                        text("state").as_deref(),
                        text("prefix").as_deref(),
                        args.integer("limit").unwrap_or(50),
                    )
                    .await
            }
            "lambda_functions" => {
                lambda
                    .functions(
                        region.as_deref(),
                        text("contains").as_deref(),
                        args.integer("limit").unwrap_or(50),
                    )
                    .await
            }
            "lambda_function" => match required("function") {
                Ok(function) => lambda.function(region.as_deref(), &function).await,
                Err(outcome) => return outcome,
            },
            "logs_filter" => {
                logs.filter(
                    region.as_deref(),
                    text("log_group").as_deref(),
                    text("function").as_deref(),
                    text("filter").as_deref(),
                    text("start").as_deref(),
                    text("end").as_deref(),
                    args.integer("limit").unwrap_or(200),
                )
                .await
            }
            "logs_groups" => {
                logs.groups(
                    region.as_deref(),
                    text("prefix").as_deref(),
                    args.integer("limit").unwrap_or(50),
                )
                .await
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
impl ToolHost for AwsMcpHost {
    fn name(&self) -> &'static str {
        "aws-mcp"
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
        self.dispatch(name, &args).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::application::test_doubles::FakeRunner;
    use crate::domain::ServerConfig;
    use crate::ports::AwsRunner;

    fn host() -> AwsMcpHost {
        let runner: Arc<dyn AwsRunner> = FakeRunner::answering(vec![
            ("--version", Ok(json!("aws-cli/2.36.24"))),
            (
                "sts get-caller-identity",
                Ok(json!({"Account": "123456789012", "Arn": "arn:x"})),
            ),
            (
                "cloudwatch get-metric-data",
                Ok(
                    json!({"MetricDataResults": [{"Id": "m", "Timestamps": ["2026-11-03T09:00:00+00:00"], "Values": [1.0]}]}),
                ),
            ),
            (
                "cloudwatch describe-alarms",
                Ok(json!({"MetricAlarms": []})),
            ),
            (
                "lambda list-functions",
                Ok(json!({"Functions": [{"FunctionName": "demo"}]})),
            ),
            (
                "lambda get-function-configuration",
                Ok(json!({"FunctionName": "demo"})),
            ),
            ("lambda list-aliases", Ok(json!({"Aliases": []}))),
            (
                "lambda list-event-source-mappings",
                Ok(json!({"EventSourceMappings": []})),
            ),
            ("lambda get-policy", Ok(json!({"Policy": "{}"}))),
            (
                "logs filter-log-events",
                Ok(json!({"events": [{"timestamp": 1, "message": "hi"}]})),
            ),
            ("logs describe-log-groups", Ok(json!({"logGroups": []}))),
        ]);
        let config = ServerConfig::new(Some("eu-west-1"), None, None).unwrap();
        AwsMcpHost::new(
            Some(QueryMetrics::new(
                runner.clone(),
                config.regions().clone(),
                None,
            )),
            Some(InspectLambda::new(
                runner.clone(),
                config.regions().clone(),
                None,
            )),
            Some(QueryLogs::new(
                runner.clone(),
                config.regions().clone(),
                None,
            )),
            Some(DiscoverScope::new(runner.clone(), config.clone())),
            DiagnoseSetup::new(Ok(config), Some(runner)),
        )
    }

    fn unconfigured() -> AwsMcpHost {
        AwsMcpHost::new(
            None,
            None,
            None,
            None,
            DiagnoseSetup::new(ServerConfig::new(Some("x"), None, None), None),
        )
    }

    fn ok(outcome: ToolOutcome) -> Value {
        match outcome {
            ToolOutcome::Ok(value) => value,
            ToolOutcome::Failed { error, .. } => panic!("failed: {error}"),
        }
    }

    #[tokio::test]
    async fn every_read_answers() {
        let host = host();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "ok");
        assert_eq!(
            ok(host.call("discover", json!({})).await)["identity"]["account"],
            "123456789012"
        );
        assert_eq!(ok(host.call("metric_query", json!({"namespace": "AWS/SQS", "metric": "V", "dimensions": {"QueueName": "q"}, "stat": "Maximum", "period": "60"})).await)["points"], 1);
        assert_eq!(
            ok(host
                .call("lambda_health", json!({"function": "demo"}))
                .await)["function"],
            "demo"
        );
        assert_eq!(
            ok(host
                .call("alarms", json!({"state": "ALARM", "prefix": "p"}))
                .await)["count"],
            0
        );
        assert_eq!(
            ok(host
                .call("lambda_functions", json!({"contains": "de"}))
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host
                .call("lambda_function", json!({"function": "demo"}))
                .await)["name"],
            "demo"
        );
        assert_eq!(
            ok(host
                .call(
                    "logs_filter",
                    json!({"function": "demo", "filter": "ERROR"})
                )
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host
                .call("logs_groups", json!({"prefix": "/aws/lambda/"}))
                .await)["count"],
            0
        );
        assert_eq!(host.tools().len(), 9);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_run() {
        let host = host();
        for (name, args, expected) in [
            ("metric_query", json!({}), "namespace is required"),
            (
                "metric_query",
                json!({"namespace": "AWS/SQS"}),
                "metric is required",
            ),
            (
                "metric_query",
                json!({"namespace": "AWS/SQS", "metric": "V", "stat": "median"}),
                "stat",
            ),
            (
                "metric_query",
                json!({"namespace": "AWS/SQS", "metric": "V", "region": "ap-south-1"}),
                "AWS_MCP_REGIONS",
            ),
            ("lambda_health", json!({}), "function is required"),
            (
                "lambda_health",
                json!({"function": "f", "period": 7}),
                "period",
            ),
            ("alarms", json!({"state": "red"}), "state"),
            ("alarms", json!({"limit": 0}), "limit"),
            ("lambda_functions", json!({"limit": 101}), "limit"),
            ("lambda_function", json!({}), "function is required"),
            ("logs_filter", json!({}), "log_group or function"),
            (
                "logs_filter",
                json!({"log_group": "/g", "start": "yesterday"}),
                "invalid time",
            ),
            ("logs_groups", json!({"prefix": "-x"}), "log_group"),
            ("unknown", json!({}), "unknown tool"),
        ] {
            let ToolOutcome::Failed { error, .. } = host.call(name, args).await else {
                panic!("{name}")
            };
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "alarms", "logs_groups"] {
            let ToolOutcome::Failed { hint, .. } = host.call(name, json!({})).await else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host()));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "aws-mcp");
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
