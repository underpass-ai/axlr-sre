use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog::{self, WRITE_TOOLS};
use crate::application::{ChangeQueues, DiagnoseSetup, DiscoverScope, InspectQueues};

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working; without
/// `MESSAGING_WRITES` the writes are not published and a call to one is
/// refused with the way to enable them.
pub struct MessagingMcpHost {
    inspect: Option<InspectQueues>,
    change: Option<ChangeQueues>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    writes: bool,
}

const INSTRUCTIONS: &str = "SQS and SNS through the aws CLI. Every call names its region or takes the first of AWS_MCP_REGIONS. Call doctor first if anything fails and discover to see who the CLI is and which queues and topics each region has. queue shows a queue's depth, redrive policy and sources; peek shows messages without deleting them (they stay hidden for a while; a queue with a redrive policy is refused). With MESSAGING_WRITES=true, send, publish, redrive, redrive_cancel and drain are published: each previews unless confirm=true, and drain writes every message to a JSON Lines file before deleting it.";

impl MessagingMcpHost {
    pub fn new(
        inspect: Option<InspectQueues>,
        change: Option<ChangeQueues>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
        writes: bool,
    ) -> Self {
        Self {
            inspect,
            change,
            discover,
            doctor,
            writes,
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
            "writes are off",
            "register messaging-mcp with MESSAGING_WRITES=true to publish send, publish, redrive, redrive_cancel and drain",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(inspect), Some(change), Some(discover)) =
            (&self.inspect, &self.change, &self.discover)
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
            "queues" => {
                inspect
                    .queues(
                        region.as_deref(),
                        text("prefix").as_deref(),
                        args.integer("limit").unwrap_or(50),
                        args.flag("with_depth", false),
                    )
                    .await
            }
            "queue" => match required("queue") {
                Ok(queue) => {
                    inspect
                        .queue(region.as_deref(), &queue, args.flag("history", true))
                        .await
                }
                Err(outcome) => return outcome,
            },
            "peek" => match required("queue") {
                Ok(queue) => {
                    inspect
                        .peek(
                            region.as_deref(),
                            &queue,
                            args.integer("limit").unwrap_or(10),
                            args.integer("visibility_seconds").unwrap_or(30),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "redrive_status" => match required("queue") {
                Ok(queue) => inspect.redrive_status(region.as_deref(), &queue).await,
                Err(outcome) => return outcome,
            },
            "topics" => {
                inspect
                    .topics(region.as_deref(), args.integer("limit").unwrap_or(50))
                    .await
            }
            "topic" => match required("topic") {
                Ok(topic) => inspect.topic(region.as_deref(), &topic).await,
                Err(outcome) => return outcome,
            },
            name if WRITE_TOOLS.contains(&name) => {
                if !self.writes {
                    return Self::read_only();
                }
                let confirm = args.flag("confirm", false);
                let group = text("message_group_id");
                let messages = args.get("messages").cloned().unwrap_or(Value::Null);
                match name {
                    "send" => match required("queue") {
                        Ok(queue) => {
                            change
                                .send(
                                    region.as_deref(),
                                    &queue,
                                    &messages,
                                    group.as_deref(),
                                    confirm,
                                )
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
                    "publish" => match required("topic") {
                        Ok(topic) => {
                            change
                                .publish(
                                    region.as_deref(),
                                    &topic,
                                    &messages,
                                    group.as_deref(),
                                    confirm,
                                )
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
                    "redrive" => match required("queue") {
                        Ok(queue) => {
                            change
                                .redrive(
                                    region.as_deref(),
                                    &queue,
                                    text("destination").as_deref(),
                                    args.integer("per_second").filter(|n| *n != 0),
                                    confirm,
                                )
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
                    "redrive_cancel" => match required("queue") {
                        Ok(queue) => {
                            change
                                .redrive_cancel(region.as_deref(), &queue, confirm)
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
                    _ => match required("queue") {
                        Ok(queue) => {
                            change
                                .drain(
                                    region.as_deref(),
                                    &queue,
                                    args.integer("limit").unwrap_or(100),
                                    args.integer("visibility_seconds").unwrap_or(60),
                                    confirm,
                                )
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
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
impl ToolHost for MessagingMcpHost {
    fn name(&self) -> &'static str {
        "messaging-mcp"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn instructions(&self) -> &'static str {
        INSTRUCTIONS
    }

    fn tools(&self) -> Vec<Value> {
        tool_catalog::tools(self.writes)
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

    use aws_mcp::domain::ServerConfig;
    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::application::QueueLookup;
    use crate::application::test_doubles::{
        FakeEvidence, FakeRunner, SOURCE, TOPIC, URL, dlq_attributes, message,
    };
    use crate::domain::MessagingConfig;

    fn host(writes: bool) -> MessagingMcpHost {
        let runner: Arc<dyn aws_mcp::ports::AwsRunner> = FakeRunner::answering(vec![
            ("--version", Ok(json!("aws-cli/2.36.24"))),
            (
                "sts get-caller-identity",
                Ok(json!({"Account": "123456789012", "Arn": "arn:x"})),
            ),
            ("sqs list-queues", Ok(json!({"QueueUrls": [URL]}))),
            ("sqs get-queue-url", Ok(json!({"QueueUrl": URL}))),
            ("sqs get-queue-attributes", Ok(dlq_attributes(1))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
            (
                "sqs receive-message",
                Ok(json!({"Messages": [message("m1", "a")]})),
            ),
            (
                "sqs delete-message-batch",
                Ok(json!({"Successful": [{"Id": "0"}]})),
            ),
            (
                "sqs send-message-batch",
                Ok(json!({"Successful": [{"Id": "0", "MessageId": "x"}]})),
            ),
            (
                "sqs list-message-move-tasks",
                Ok(json!({"Results": [{"TaskHandle": "h", "Status": "RUNNING"}]})),
            ),
            (
                "sqs cancel-message-move-task",
                Ok(json!({"ApproximateNumberOfMessagesMoved": 1})),
            ),
            (
                "cloudwatch get-metric-data",
                Ok(json!({"MetricDataResults": []})),
            ),
            (
                "sns list-topics",
                Ok(json!({"Topics": [{"TopicArn": TOPIC}]})),
            ),
            ("sns get-topic-attributes", Ok(json!({"Attributes": {}}))),
            (
                "sns list-subscriptions-by-topic",
                Ok(json!({"Subscriptions": []})),
            ),
            (
                "sns publish-batch",
                Ok(json!({"Successful": [{"Id": "0", "MessageId": "y"}]})),
            ),
        ]);
        let config = MessagingConfig::new(
            ServerConfig::new(Some("eu-west-1"), None, None).unwrap(),
            Some(if writes { "true" } else { "false" }),
            Some("/snap"),
            None,
        )
        .unwrap();
        let lookup = Arc::new(QueueLookup::new(runner.clone(), None));
        let regions = config.aws().regions().clone();
        MessagingMcpHost::new(
            Some(InspectQueues::new(lookup.clone(), regions.clone())),
            Some(ChangeQueues::new(
                lookup.clone(),
                regions,
                writes,
                Arc::new(FakeEvidence::default()),
            )),
            Some(DiscoverScope::new(lookup, config.clone())),
            DiagnoseSetup::new(Ok(config), Some(runner)),
            writes,
        )
    }

    fn unconfigured() -> MessagingMcpHost {
        MessagingMcpHost::new(
            None,
            None,
            None,
            DiagnoseSetup::new(Err(crate::domain::ConfigError::InvalidWrites), None),
            false,
        )
    }

    fn ok(outcome: ToolOutcome) -> Value {
        match outcome {
            ToolOutcome::Ok(value) => value,
            ToolOutcome::Failed { error, .. } => panic!("failed: {error}"),
        }
    }

    #[tokio::test]
    async fn every_read_answers_and_the_writes_preview_then_run() {
        let host = host(true);
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "ok");
        assert_eq!(
            ok(host.call("discover", json!({})).await)["identity"]["account"],
            "123456789012"
        );
        assert_eq!(
            ok(host.call("queues", json!({"with_depth": true})).await)["count"],
            1
        );
        assert_eq!(
            ok(host
                .call("queue", json!({"queue": "demo-orders-dlq"}))
                .await)["isDeadLetter"],
            true
        );
        assert_eq!(
            ok(host.call("peek", json!({"queue": URL, "limit": 1})).await)["summary"]["count"],
            1
        );
        assert_eq!(
            ok(host.call("redrive_status", json!({"queue": URL})).await)["running"],
            true
        );
        assert_eq!(ok(host.call("topics", json!({})).await)["count"], 1);
        assert_eq!(
            ok(host.call("topic", json!({"topic": TOPIC})).await)["name"],
            "demo-topic"
        );
        assert_eq!(
            ok(host
                .call("send", json!({"queue": URL, "messages": [{"body": "x"}]}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call(
                    "send",
                    json!({"queue": URL, "messages": [{"body": "x"}], "confirm": true})
                )
                .await)["sent"],
            1
        );
        assert_eq!(
            ok(host
                .call(
                    "publish",
                    json!({"topic": TOPIC, "messages": [{"body": "x"}], "confirm": true})
                )
                .await)["sent"],
            1
        );
        let ToolOutcome::Failed { error, .. } = host
            .call("redrive", json!({"queue": URL, "per_second": 0}))
            .await
        else {
            panic!()
        };
        assert!(error.contains("already RUNNING"), "{error}");
        assert_eq!(
            ok(host
                .call("redrive_cancel", json!({"queue": URL, "confirm": true}))
                .await)["moved"],
            1
        );
        let ToolOutcome::Failed { error, .. } = host.call("drain", json!({"queue": URL})).await
        else {
            panic!()
        };
        assert!(error.contains("already RUNNING"), "{error}");
        assert_eq!(host.tools().len(), 13);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_run() {
        let host = host(true);
        for (name, args, expected) in [
            ("queues", json!({"limit": 0}), "limit"),
            ("queue", json!({}), "queue is required"),
            ("queue", json!({"queue": "bad name"}), "invalid queue"),
            ("peek", json!({"queue": URL, "limit": 101}), "limit"),
            ("redrive_status", json!({}), "queue is required"),
            ("topics", json!({"limit": 0}), "limit"),
            ("topic", json!({}), "topic is required"),
            ("topic", json!({"topic": "nope"}), "ARN"),
            ("send", json!({"queue": URL}), "messages"),
            ("send", json!({"messages": []}), "queue is required"),
            (
                "publish",
                json!({"messages": [{"body": "x"}]}),
                "topic is required",
            ),
            ("redrive", json!({}), "queue is required"),
            ("redrive_cancel", json!({}), "queue is required"),
            ("drain", json!({"queue": URL, "limit": 0}), "limit"),
            ("unknown", json!({}), "unknown tool"),
        ] {
            let ToolOutcome::Failed { error, .. } = host.call(name, args).await else {
                panic!("{name}")
            };
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn without_writes_the_write_tools_are_hidden_and_refused() {
        let host = host(false);
        assert_eq!(host.tools().len(), 8);
        let ToolOutcome::Failed { hint, .. } = host
            .call("drain", json!({"queue": URL, "confirm": true}))
            .await
        else {
            panic!()
        };
        assert!(hint.unwrap().contains("MESSAGING_WRITES=true"));
        assert_eq!(ok(host.call("queues", json!({})).await)["count"], 1);
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "queues", "drain"] {
            let ToolOutcome::Failed { hint, .. } = host.call(name, json!({"queue": URL})).await
            else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host(false)));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "messaging-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("MESSAGING_WRITES")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 8);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
