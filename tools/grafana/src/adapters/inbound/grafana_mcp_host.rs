use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog;
use crate::application::{DiagnoseSetup, DiscoverScope, InspectGrafana, QueryLogs, QueryMetrics};
use crate::domain::Direction;

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working.
pub struct GrafanaMcpHost {
    logs: Option<QueryLogs>,
    metrics: Option<QueryMetrics>,
    inspect: Option<InspectGrafana>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
}

const INSTRUCTIONS: &str = "Read-only access to Grafana with a service account token: Loki logs, Prometheus metrics, dashboards and alerts, all through Grafana's API. Call doctor first if anything fails and discover to see the datasources, the dashboards and the alert counts. Windows take RFC 3339 or how long ago (15m, 2h, 1d), seven days at most; answers are bounded and a log page carries next.end to page further back. Nothing here writes.";

impl GrafanaMcpHost {
    pub fn new(
        logs: Option<QueryLogs>,
        metrics: Option<QueryMetrics>,
        inspect: Option<InspectGrafana>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
    ) -> Self {
        Self {
            logs,
            metrics,
            inspect,
            discover,
            doctor,
        }
    }

    fn not_configured() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "the server is not configured",
            "call doctor: it names the variable that is missing",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(logs), Some(metrics), Some(inspect), Some(discover)) =
            (&self.logs, &self.metrics, &self.inspect, &self.discover)
        else {
            return Self::not_configured();
        };
        let text = |key: &str| args.text(key);
        let required = |key: &str| {
            args.text(key)
                .ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let result = match name {
            "discover" => discover.execute().await,
            "loki_query" => {
                let query = match required("query") {
                    Ok(query) => query,
                    Err(outcome) => return outcome,
                };
                let direction =
                    match Direction::parse(&text("direction").unwrap_or_else(|| "backward".into()))
                    {
                        Ok(direction) => direction,
                        Err(error) => return ToolOutcome::failed(error.to_string(), None),
                    };
                logs.query(
                    text("datasource").as_deref(),
                    &query,
                    text("start").as_deref(),
                    text("end").as_deref(),
                    args.integer("limit").unwrap_or(100),
                    direction,
                )
                .await
            }
            "loki_labels" => {
                logs.labels(
                    text("datasource").as_deref(),
                    text("start").as_deref(),
                    text("end").as_deref(),
                )
                .await
            }
            "loki_label_values" => match required("label") {
                Ok(label) => {
                    logs.label_values(
                        text("datasource").as_deref(),
                        &label,
                        text("start").as_deref(),
                        text("end").as_deref(),
                    )
                    .await
                }
                Err(outcome) => return outcome,
            },
            "metrics_query" => match required("query") {
                Ok(query) => {
                    metrics
                        .instant(
                            text("datasource").as_deref(),
                            &query,
                            text("time").as_deref(),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "metrics_range" => match required("query") {
                Ok(query) => {
                    metrics
                        .range(
                            text("datasource").as_deref(),
                            &query,
                            text("start").as_deref(),
                            text("end").as_deref(),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "dashboards" => {
                inspect
                    .dashboards(
                        text("query").as_deref(),
                        text("tag").as_deref(),
                        args.integer("limit").unwrap_or(25),
                    )
                    .await
            }
            "dashboard" => match required("uid") {
                Ok(uid) => {
                    inspect
                        .dashboard(&uid, args.flag("with_queries", true))
                        .await
                }
                Err(outcome) => return outcome,
            },
            "alerts" => {
                inspect
                    .alerts(
                        text("datasource").as_deref(),
                        args.flag("firing_only", true),
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
impl ToolHost for GrafanaMcpHost {
    fn name(&self) -> &'static str {
        "grafana-mcp"
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
    use crate::application::Datasources;
    use crate::application::test_doubles::FakeReader;
    use crate::domain::ServerConfig;
    use crate::ports::GrafanaReader;

    fn host() -> GrafanaMcpHost {
        let reader: Arc<dyn GrafanaReader> = FakeReader::answering(vec![
            (
                "/api/health",
                Ok(json!({"version": "12", "database": "ok"})),
            ),
            ("/api/org", Ok(json!({"name": "o"}))),
            (
                "/api/datasources/proxy/uid/l/loki/api/v1/query_range",
                Ok(json!({"data": {"result": [{"stream": {}, "values": [["1", "line"]]}]}})),
            ),
            (
                "/api/datasources/proxy/uid/l/loki/api/v1/labels",
                Ok(json!({"data": ["app"]})),
            ),
            (
                "/api/datasources/proxy/uid/l/loki/api/v1/label",
                Ok(json!({"data": ["web"]})),
            ),
            (
                "/api/datasources/proxy/uid/p/api/v1/query_range",
                Ok(json!({"data": {"resultType": "matrix", "result": []}})),
            ),
            (
                "/api/datasources/proxy/uid/p/api/v1/query",
                Ok(json!({"data": {"resultType": "vector", "result": []}})),
            ),
            (
                "/api/datasources",
                Ok(
                    json!([{"uid": "l", "name": "L", "type": "loki"}, {"uid": "p", "name": "P", "type": "prometheus"}]),
                ),
            ),
            ("/api/search", Ok(json!([{"uid": "d", "title": "D"}]))),
            (
                "/api/dashboards/uid/d",
                Ok(json!({"dashboard": {"uid": "d", "panels": []}})),
            ),
            ("/api/alertmanager/grafana", Ok(json!([]))),
        ]);
        let datasources = Arc::new(Datasources::new(reader.clone(), None, None));
        let config = ServerConfig::new(Some("grafana.example.com"), Some("t"), None, None);
        GrafanaMcpHost::new(
            Some(QueryLogs::new(reader.clone(), datasources.clone())),
            Some(QueryMetrics::new(reader.clone(), datasources.clone())),
            Some(InspectGrafana::new(reader.clone())),
            Some(DiscoverScope::new(reader.clone(), datasources)),
            DiagnoseSetup::new(config, Some(reader)),
        )
    }

    fn unconfigured() -> GrafanaMcpHost {
        GrafanaMcpHost::new(
            None,
            None,
            None,
            None,
            DiagnoseSetup::new(ServerConfig::new(None, None, None, None), None),
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
            ok(host.call("discover", json!({})).await)["datasources"]["count"],
            2
        );
        assert_eq!(
            ok(host
                .call("loki_query", json!({"query": "{a=\"b\"}", "limit": "5"}))
                .await)["count"],
            1
        );
        assert_eq!(
            ok(host
                .call(
                    "loki_query",
                    json!({"query": "{a=\"b\"}", "direction": "forward"})
                )
                .await)["direction"],
            "forward"
        );
        assert_eq!(
            ok(host.call("loki_labels", json!({})).await)["values"],
            json!(["app"])
        );
        assert_eq!(
            ok(host
                .call("loki_label_values", json!({"label": "app"}))
                .await)["values"],
            json!(["web"])
        );
        assert_eq!(
            ok(host.call("metrics_query", json!({"query": "up"})).await)["type"],
            "vector"
        );
        assert_eq!(
            ok(host
                .call("metrics_range", json!({"query": "up", "start": "2h"}))
                .await)["type"],
            "matrix"
        );
        assert_eq!(
            ok(host.call("dashboards", json!({"query": "D"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("dashboard", json!({"uid": "d"})).await)["uid"],
            "d"
        );
        assert_eq!(ok(host.call("alerts", json!({})).await)["count"], 0);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_request() {
        let host = host();
        for (name, args, expected) in [
            ("loki_query", json!({}), "query is required"),
            ("loki_query", json!({"query": "x"}), "stream selector"),
            (
                "loki_query",
                json!({"query": "{a=\"b\"}", "direction": "up"}),
                "direction",
            ),
            (
                "loki_query",
                json!({"query": "{a=\"b\"}", "limit": 0}),
                "limit",
            ),
            (
                "loki_query",
                json!({"query": "{a=\"b\"}", "start": "tomorrow"}),
                "invalid time",
            ),
            ("loki_label_values", json!({}), "label is required"),
            ("metrics_query", json!({}), "query is required"),
            ("metrics_range", json!({}), "query is required"),
            (
                "metrics_query",
                json!({"query": "up", "datasource": "a b"}),
                "uid",
            ),
            ("dashboards", json!({"limit": 101}), "limit"),
            ("dashboard", json!({}), "uid is required"),
            ("alerts", json!({"datasource": "a/b"}), "uid"),
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
        for name in ["discover", "loki_query", "dashboards"] {
            let ToolOutcome::Failed { hint, .. } =
                host.call(name, json!({"query": "{a=\"b\"}"})).await
            else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host()));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "grafana-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("Read-only")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 10);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
