use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog::{self, WRITE_TOOLS};
use crate::application::{
    ChangeInstance, DiagnoseSetup, DiscoverScope, InspectInstance, InspectKeys,
};

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working; without `REDIS_WRITES`
/// the writes are not published and a call to one is refused with the way
/// to enable them.
pub struct RedisMcpHost {
    instance: Option<InspectInstance>,
    keys: Option<InspectKeys>,
    change: Option<ChangeInstance>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    writes: bool,
}

const INSTRUCTIONS: &str = "One Redis or Valkey instance, reached with this node's environment. Call doctor first if anything fails and discover for the instance at a glance. info, slowlog, clients, latency, memory, config_get and cluster are the DBA's questions; keys walks SCAN (never KEYS) with bounded previews, key details one key, biggest measures a sample. With REDIS_WRITES=true, expire, delete, config_set (an allowlist of parameters) and client_kill are published: each previews unless confirm=true. FLUSHALL, FLUSHDB, SHUTDOWN, DEBUG and data writes have no tool here.";

impl RedisMcpHost {
    pub fn new(
        instance: Option<InspectInstance>,
        keys: Option<InspectKeys>,
        change: Option<ChangeInstance>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
        writes: bool,
    ) -> Self {
        Self {
            instance,
            keys,
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
            "register redis-mcp with REDIS_WRITES=true to publish expire, delete, config_set and client_kill",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(instance), Some(keys), Some(change), Some(discover)) =
            (&self.instance, &self.keys, &self.change, &self.discover)
        else {
            return Self::not_configured();
        };
        let text = |key: &str| args.text(key).filter(|t| !t.trim().is_empty());
        let required = |key: &str| {
            text(key).ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let result = match name {
            "discover" => discover.execute().await,
            "info" => {
                instance
                    .info(&text("section").unwrap_or_else(|| "default".into()))
                    .await
            }
            "slowlog" => instance.slowlog(args.integer("count").unwrap_or(25)).await,
            "clients" => instance.clients().await,
            "latency" => instance.latency().await,
            "memory" => instance.memory().await,
            "config_get" => match required("pattern") {
                Ok(pattern) => instance.config_get(&pattern).await,
                Err(outcome) => return outcome,
            },
            "cluster" => instance.cluster().await,
            "keys" => {
                keys.keys(
                    text("pattern").as_deref(),
                    text("value_contains").as_deref(),
                    args.flag("ignore_case", false),
                    args.flag("with_values", false),
                    args.integer("limit").unwrap_or(100),
                )
                .await
            }
            "key" => match required("key") {
                Ok(key) => keys.key(&key).await,
                Err(outcome) => return outcome,
            },
            "biggest" => {
                keys.biggest(
                    text("pattern").as_deref(),
                    args.integer("limit").unwrap_or(20),
                    args.integer("sample").unwrap_or(5000),
                )
                .await
            }
            name if WRITE_TOOLS.contains(&name) => {
                if !self.writes {
                    return Self::read_only();
                }
                let confirm = args.flag("confirm", false);
                match name {
                    "expire" => match (required("key"), args.integer("seconds")) {
                        (Ok(key), Some(seconds)) => change.expire(&key, seconds, confirm).await,
                        (Err(outcome), _) => return outcome,
                        (_, None) => return ToolOutcome::failed("seconds is required", None),
                    },
                    "delete" => {
                        change
                            .delete(
                                &args.texts("keys"),
                                text("pattern").as_deref(),
                                text("value_contains").as_deref(),
                                args.flag("ignore_case", false),
                                confirm,
                            )
                            .await
                    }
                    "config_set" => match (required("parameter"), required("value")) {
                        (Ok(parameter), Ok(value)) => {
                            change.config_set(&parameter, &value, confirm).await
                        }
                        (Err(outcome), _) | (_, Err(outcome)) => return outcome,
                    },
                    _ => match args.integer("id") {
                        Some(id) if id > 0 => change.client_kill(id, confirm).await,
                        _ => return ToolOutcome::failed("id is required", None),
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
impl ToolHost for RedisMcpHost {
    fn name(&self) -> &'static str {
        "redis-mcp"
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

    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::application::test_doubles::{FakeGateway, text, texts};
    use crate::domain::{ConfigError, ConnectionConfig, Reply, ServerConfig};

    fn host(writes: bool) -> RedisMcpHost {
        let gateway = FakeGateway::answering(vec![
            ("PING", Ok(text("PONG"))),
            (
                "INFO",
                Ok(text(
                    "# Server\r\nredis_version:7.2\r\n# Memory\r\nused_memory:1\r\nmaxmemory:0\r\n# Replication\r\nrole:master\r\n# Cluster\r\ncluster_enabled:0\r\n# Keyspace\r\ndb0:keys=1\r\n",
                )),
            ),
            ("DBSIZE", Ok(Reply::Int(1))),
            ("SLOWLOG GET", Ok(Reply::Array(vec![]))),
            ("SLOWLOG LEN", Ok(Reply::Int(0))),
            ("CONFIG GET", Ok(texts(&["maxmemory", "0"]))),
            ("CLIENT LIST ID 7", Ok(text("id=7 flags=N cmd=get\n"))),
            ("CLIENT LIST", Ok(text("id=1 flags=N cmd=client\n"))),
            ("CLIENT ID", Ok(Reply::Int(1))),
            ("CLIENT KILL ID 7", Ok(Reply::Int(1))),
            ("LATENCY LATEST", Ok(Reply::Array(vec![]))),
            ("MEMORY DOCTOR", Ok(text("fine"))),
            ("MEMORY STATS", Ok(texts(&[]))),
            (
                "SCAN",
                Ok(Reply::Array(vec![text("0"), texts(&["demo:a"])])),
            ),
            ("TYPE demo:a", Ok(text("string"))),
            ("TTL demo:a", Ok(Reply::Int(10))),
            ("OBJECT ENCODING", Ok(text("embstr"))),
            ("MEMORY USAGE", Ok(Reply::Int(50))),
            ("GETRANGE", Ok(text("v"))),
            ("OBJECT IDLETIME", Ok(Reply::Int(1))),
            ("OBJECT FREQ", Ok(Reply::Int(1))),
            ("EXPIRE", Ok(Reply::Int(1))),
            ("UNLINK", Ok(Reply::Int(1))),
        ]);
        let connection =
            ConnectionConfig::from_vars(|name| (name == "REDIS_HOST").then(|| "h".into())).unwrap();
        let config = ServerConfig::new(connection.clone(), writes);
        RedisMcpHost::new(
            Some(InspectInstance::new(gateway.clone())),
            Some(InspectKeys::new(gateway.clone())),
            Some(ChangeInstance::new(gateway.clone(), writes)),
            Some(DiscoverScope::new(
                gateway.clone(),
                writes,
                connection.display(),
            )),
            DiagnoseSetup::new(Ok(config), Some(gateway)),
            writes,
        )
    }

    fn unconfigured() -> RedisMcpHost {
        RedisMcpHost::new(
            None,
            None,
            None,
            None,
            DiagnoseSetup::new(Err(ConfigError::MissingConnection), None),
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
        assert_eq!(ok(host.call("discover", json!({})).await)["dbsize"], 1);
        assert_eq!(
            ok(host.call("info", json!({"section": "server"})).await)["sections"]["server"]["redis_version"],
            "7.2"
        );
        assert_eq!(ok(host.call("slowlog", json!({})).await)["count"], 0);
        assert_eq!(ok(host.call("clients", json!({})).await)["count"], 1);
        assert_eq!(ok(host.call("latency", json!({})).await)["count"], 0);
        assert_eq!(ok(host.call("memory", json!({})).await)["doctor"], "fine");
        assert_eq!(
            ok(host
                .call("config_get", json!({"pattern": "maxmemory"}))
                .await)["count"],
            1
        );
        assert_eq!(ok(host.call("cluster", json!({})).await)["enabled"], false);
        assert_eq!(
            ok(host
                .call("keys", json!({"pattern": "demo:*", "with_values": true}))
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("key", json!({"key": "demo:a"})).await)["exists"],
            true
        );
        assert_eq!(
            ok(host.call("biggest", json!({"limit": 5})).await)["sampled"],
            1
        );
        assert_eq!(
            ok(host
                .call("expire", json!({"key": "demo:a", "seconds": 60}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call(
                    "expire",
                    json!({"key": "demo:a", "seconds": 60, "confirm": true})
                )
                .await)["set"],
            true
        );
        assert_eq!(
            ok(host
                .call("delete", json!({"keys": ["demo:a"], "confirm": true}))
                .await)["deleted"],
            1
        );
        assert_eq!(
            ok(host.call("delete", json!({"pattern": "demo:*"})).await)["selected"],
            1
        );
        assert_eq!(
            ok(host
                .call(
                    "config_set",
                    json!({"parameter": "maxmemory", "value": "64mb"})
                )
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call("client_kill", json!({"id": 7, "confirm": true}))
                .await)["killed"],
            1
        );
        assert_eq!(host.tools().len(), 16);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_command() {
        let host = host(true);
        for (name, args, expected) in [
            ("info", json!({"section": "secrets"}), "section"),
            ("slowlog", json!({"count": 0}), "count"),
            ("config_get", json!({}), "pattern is required"),
            ("keys", json!({"pattern": "*"}), "every key"),
            ("keys", json!({"pattern": "demo:*", "limit": 0}), "limit"),
            ("key", json!({}), "key is required"),
            ("biggest", json!({"limit": 0}), "limit"),
            ("expire", json!({"key": "k"}), "seconds is required"),
            ("expire", json!({"seconds": 1}), "key is required"),
            ("delete", json!({}), "every key"),
            (
                "config_set",
                json!({"parameter": "dir", "value": "/x"}),
                "not a parameter",
            ),
            (
                "config_set",
                json!({"parameter": "maxmemory"}),
                "value is required",
            ),
            ("client_kill", json!({}), "id is required"),
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
        assert_eq!(host.tools().len(), 12);
        let ToolOutcome::Failed { hint, .. } = host
            .call("delete", json!({"keys": ["k"], "confirm": true}))
            .await
        else {
            panic!()
        };
        assert!(hint.unwrap().contains("REDIS_WRITES=true"));
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "keys", "delete"] {
            let ToolOutcome::Failed { hint, .. } = host.call(name, json!({})).await else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host(false)));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "redis-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("REDIS_WRITES")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 12);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
