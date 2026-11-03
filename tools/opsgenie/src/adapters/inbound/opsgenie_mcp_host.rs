use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog::{self, WRITE_TOOLS};
use crate::application::{ChangeAlerts, DiagnoseSetup, DiscoverScope, InspectAlerts};

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working; without
/// `OPSGENIE_WRITES` the writes are not published and a call to one is
/// refused with the way to enable them.
pub struct OpsgenieMcpHost {
    inspect: Option<InspectAlerts>,
    change: Option<ChangeAlerts>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    writes: bool,
}

const INSTRUCTIONS: &str = "Opsgenie through its REST API with an API key, within a scope of teams and escalations: alerts outside it are neither shown nor changed. Call doctor first if anything fails and discover to see the scope and how many alerts are open. alerts_open lists them newest first; alert and alert_notes detail one. With OPSGENIE_WRITES=true, alert_acknowledge, alert_note and alert_close are published: each previews unless confirm=true, and Opsgenie processes writes asynchronously, so a done answer says whether it was already verified.";

impl OpsgenieMcpHost {
    pub fn new(
        inspect: Option<InspectAlerts>,
        change: Option<ChangeAlerts>,
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
            "call doctor: it names the variable that is missing",
        )
    }

    fn read_only() -> ToolOutcome {
        ToolOutcome::failed_with_hint(
            "writes are off",
            "register opsgenie-mcp with OPSGENIE_WRITES=true to publish alert_acknowledge, alert_note and alert_close",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(inspect), Some(change), Some(discover)) =
            (&self.inspect, &self.change, &self.discover)
        else {
            return Self::not_configured();
        };
        let required = |key: &str| {
            args.text(key)
                .ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let result = match name {
            "discover" => discover.execute().await,
            "alerts_open" => {
                inspect
                    .alerts_open(
                        args.integer("limit").unwrap_or(20),
                        args.integer("offset").unwrap_or(0),
                    )
                    .await
            }
            "alert" => match required("alert") {
                Ok(alert) => inspect.alert(&alert).await,
                Err(outcome) => return outcome,
            },
            "alert_notes" => match required("alert") {
                Ok(alert) => {
                    inspect
                        .alert_notes(&alert, args.integer("limit").unwrap_or(20))
                        .await
                }
                Err(outcome) => return outcome,
            },
            name if WRITE_TOOLS.contains(&name) => {
                if !self.writes {
                    return Self::read_only();
                }
                let alert = match required("alert") {
                    Ok(alert) => alert,
                    Err(outcome) => return outcome,
                };
                let note = args.text("note");
                let confirm = args.flag("confirm", false);
                match name {
                    "alert_acknowledge" => {
                        change.acknowledge(&alert, note.as_deref(), confirm).await
                    }
                    "alert_note" => change.note(&alert, note.as_deref(), confirm).await,
                    _ => change.close(&alert, note.as_deref(), confirm).await,
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
impl ToolHost for OpsgenieMcpHost {
    fn name(&self) -> &'static str {
        "opsgenie-mcp"
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
    use crate::application::ReadAlerts;
    use crate::application::test_doubles::{FakeClient, listings, ours, page, theirs};
    use crate::domain::{Scope, ServerConfig};
    use crate::ports::OpsgenieClient;

    fn host(writes: bool) -> OpsgenieMcpHost {
        let mut answers = listings();
        answers.push(("/v2/alerts/count", Ok(json!({"data": {"count": 2}}))));
        answers.push(("/v2/alerts/1", Ok(json!({"data": ours(1, "P1")}))));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/notes",
            Ok(json!({"data": [{"note": "n"}]})),
        ));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/close",
            Ok(json!({"requestId": "r"})),
        ));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/acknowledge",
            Ok(json!({"requestId": "r"})),
        ));
        answers.push((
            "/v2/alerts/requests/r",
            Ok(json!({"data": {"isSuccess": true}})),
        ));
        answers.push(page(vec![ours(1, "P1"), theirs(2)]));
        let client: Arc<dyn OpsgenieClient> = FakeClient::answering(answers);
        let config = ServerConfig::new(
            None,
            Some("k"),
            Some("platform"),
            None,
            Some(if writes { "true" } else { "false" }),
            None,
        );
        let read = Arc::new(ReadAlerts::new(
            client.clone(),
            Scope::new(Some("platform"), None).unwrap(),
        ));
        OpsgenieMcpHost::new(
            Some(InspectAlerts::new(client.clone(), read.clone())),
            Some(ChangeAlerts::new(
                client.clone(),
                read.clone(),
                writes,
                None,
            )),
            Some(DiscoverScope::new(read.clone(), "s".into(), writes)),
            DiagnoseSetup::new(config, Some(client), Some(read)),
            writes,
        )
    }

    fn unconfigured() -> OpsgenieMcpHost {
        OpsgenieMcpHost::new(
            None,
            None,
            None,
            DiagnoseSetup::new(
                ServerConfig::new(None, None, None, None, None, None),
                None,
                None,
            ),
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
            ok(host.call("discover", json!({})).await)["open"]["inScope"],
            1
        );
        assert_eq!(
            ok(host.call("alerts_open", json!({"limit": "5"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("alert", json!({"alert": "#1"})).await)["tinyId"],
            "1"
        );
        assert_eq!(
            ok(host.call("alert_notes", json!({"alert": "1"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host
                .call("alert_close", json!({"alert": "1", "note": "fixed"}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call(
                    "alert_close",
                    json!({"alert": "1", "note": "fixed", "confirm": true})
                )
                .await)["processed"],
            "verified"
        );
        assert_eq!(
            ok(host
                .call("alert_note", json!({"alert": "1", "note": "n"}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call("alert_acknowledge", json!({"alert": "1", "confirm": true}))
                .await)["mode"],
            "done"
        );
        assert_eq!(host.tools().len(), 8);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_request() {
        let host = host(true);
        for (name, args, expected) in [
            ("alerts_open", json!({"limit": 0}), "limit"),
            ("alert", json!({}), "alert is required"),
            ("alert", json!({"alert": "x"}), "invalid alert"),
            ("alert_notes", json!({}), "alert is required"),
            ("alert_notes", json!({"alert": "1", "limit": 0}), "limit"),
            ("alert_close", json!({}), "alert is required"),
            ("alert_close", json!({"alert": "1"}), "note is required"),
            (
                "alert_note",
                json!({"alert": "1", "note": " "}),
                "note is required",
            ),
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
        assert_eq!(host.tools().len(), 5);
        let ToolOutcome::Failed { hint, .. } = host
            .call(
                "alert_close",
                json!({"alert": "1", "note": "x", "confirm": true}),
            )
            .await
        else {
            panic!()
        };
        assert!(hint.unwrap().contains("OPSGENIE_WRITES=true"));
        assert_eq!(ok(host.call("alerts_open", json!({})).await)["total"], 1);
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "alerts_open", "alert_close"] {
            let ToolOutcome::Failed { hint, .. } = host.call(name, json!({"alert": "1"})).await
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
        assert_eq!(init["result"]["serverInfo"]["name"], "opsgenie-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("scope")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
