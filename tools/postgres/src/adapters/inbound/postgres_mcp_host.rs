use async_trait::async_trait;
use serde_json::Value;
use sre_mcp::{Arguments, ToolHost, ToolOutcome};

use super::tool_catalog::{self, WRITE_TOOLS};
use crate::application::{ChangeServer, DiagnoseSetup, DiscoverScope, InspectServer, RunQuery};
use crate::domain::WriteLevel;

/// The MCP entry: reads the arguments and hands the call to a use case. A
/// configuration error leaves only `doctor` working; without
/// `POSTGRES_WRITES` the writes are not published and a call to one is
/// refused with the way to enable them.
pub struct PostgresMcpHost {
    query: Option<RunQuery>,
    inspect: Option<InspectServer>,
    change: Option<ChangeServer>,
    discover: Option<DiscoverScope>,
    doctor: DiagnoseSetup,
    writes: WriteLevel,
}

const INSTRUCTIONS: &str = "One PostgreSQL database, reached with the libpq environment of this node. Call doctor first if anything fails and discover to see the schemas, the largest tables and the settings. query runs one read in a read-only transaction with a timeout and a row cap; explain shows a plan; activity, locks, table_stats, index_stats, statements, settings and replication are the DBA's questions, ready-made. With POSTGRES_WRITES=maintenance, execute runs VACUUM, ANALYZE, REINDEX, CREATE INDEX CONCURRENTLY and ALTER; with all, any statement and terminate too: each previews unless confirm=true. DROP, TRUNCATE, DELETE, UPDATE and INSERT stay with a person unless the level is all.";

impl PostgresMcpHost {
    pub fn new(
        query: Option<RunQuery>,
        inspect: Option<InspectServer>,
        change: Option<ChangeServer>,
        discover: Option<DiscoverScope>,
        doctor: DiagnoseSetup,
        writes: WriteLevel,
    ) -> Self {
        Self {
            query,
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
            "register postgres-mcp with POSTGRES_WRITES=maintenance or all to publish execute and terminate",
        )
    }

    async fn dispatch(&self, name: &str, args: &Arguments) -> ToolOutcome {
        let (Some(query), Some(inspect), Some(change), Some(discover)) =
            (&self.query, &self.inspect, &self.change, &self.discover)
        else {
            return Self::not_configured();
        };
        let text = |key: &str| args.text(key).filter(|t| !t.trim().is_empty());
        let required = |key: &str| {
            text(key).ok_or_else(|| ToolOutcome::failed(format!("{key} is required"), None))
        };
        let result = match name {
            "discover" => discover.execute(text("schema").as_deref()).await,
            "query" => match required("sql") {
                Ok(sql) => {
                    query
                        .query(
                            &sql,
                            args.integer("max_rows").unwrap_or(200),
                            args.integer("timeout_seconds").unwrap_or(30),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "explain" => match required("sql") {
                Ok(sql) => {
                    query
                        .explain(
                            &sql,
                            args.flag("analyze", false),
                            &text("format").unwrap_or_else(|| "text".into()),
                            args.integer("timeout_seconds").unwrap_or(30),
                        )
                        .await
                }
                Err(outcome) => return outcome,
            },
            "activity" => {
                inspect
                    .activity(
                        args.flag("include_idle", false),
                        args.integer("limit").unwrap_or(50),
                    )
                    .await
            }
            "locks" => inspect.locks(args.integer("limit").unwrap_or(50)).await,
            "table_stats" => {
                inspect
                    .table_stats(
                        text("schema").as_deref(),
                        &text("order").unwrap_or_else(|| "dead".into()),
                        args.integer("limit").unwrap_or(30),
                    )
                    .await
            }
            "index_stats" => {
                inspect
                    .index_stats(
                        text("schema").as_deref(),
                        args.flag("unused_only", false),
                        args.integer("limit").unwrap_or(30),
                    )
                    .await
            }
            "statements" => {
                inspect
                    .statements(
                        &text("order").unwrap_or_else(|| "total".into()),
                        args.integer("limit").unwrap_or(20),
                    )
                    .await
            }
            "settings" => {
                inspect
                    .settings(&args.texts("names"), text("prefix").as_deref())
                    .await
            }
            "replication" => inspect.replication().await,
            name if WRITE_TOOLS.contains(&name) => {
                if self.writes == WriteLevel::Off {
                    return Self::read_only();
                }
                let confirm = args.flag("confirm", false);
                match name {
                    "execute" => match required("sql") {
                        Ok(sql) => {
                            change
                                .execute(
                                    &sql,
                                    args.integer("timeout_seconds").unwrap_or(60),
                                    confirm,
                                )
                                .await
                        }
                        Err(outcome) => return outcome,
                    },
                    _ => match args.integer("pid") {
                        Some(pid) if pid > 0 => {
                            change
                                .terminate(pid, args.flag("cancel", false), confirm)
                                .await
                        }
                        _ => return ToolOutcome::failed("pid is required", None),
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
impl ToolHost for PostgresMcpHost {
    fn name(&self) -> &'static str {
        "postgres-mcp"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn instructions(&self) -> &'static str {
        INSTRUCTIONS
    }

    fn tools(&self) -> Vec<Value> {
        tool_catalog::tools(self.writes != WriteLevel::Off)
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
    use std::path::Path;
    use std::sync::Arc;

    use serde_json::json;
    use sre_mcp::StdioServer;

    use super::*;
    use crate::application::test_doubles::{FakeGateway, command, rows};
    use crate::domain::{ConfigError, ConnectionConfig, Files, ServerConfig};

    struct NoFiles;
    impl Files for NoFiles {
        fn read(&self, _: &Path) -> Option<(String, u32)> {
            None
        }
    }

    fn host(level: WriteLevel) -> PostgresMcpHost {
        let gateway = FakeGateway::answering(vec![
            (
                "version()",
                rows(
                    &["version", "user", "database", "in_recovery"],
                    &[&["PostgreSQL 16.4", "app", "orders", "f"]],
                ),
            ),
            ("FROM pg_roles", rows(&["superuser"], &[&["f"]])),
            (
                "information_schema.columns",
                rows(&["column_name"], &[&["total_exec_time"]]),
            ),
            ("pg_database_size", rows(&["size"], &[&["1 MB"]])),
            ("FROM pg_namespace", rows(&["schema"], &[&["demo"]])),
            (
                "pg_total_relation_size(c.oid) DESC",
                rows(&["table"], &[&["orders"]]),
            ),
            (
                "FROM pg_extension",
                rows(&["name", "version"], &[&["plpgsql", "1.0"]]),
            ),
            ("FETCH", rows(&["one"], &[&["1"]])),
            ("EXPLAIN (", rows(&["QUERY PLAN"], &[&["Result"]])),
            ("EXPLAIN update", rows(&["QUERY PLAN"], &[&["Update"]])),
            ("backend_type = 'client backend'", rows(&["pid"], &[&["7"]])),
            (
                "WHERE pid = 7",
                rows(
                    &["pid", "query", "backend_type"],
                    &[&["7", "select 1", "client backend"]],
                ),
            ),
            (
                "pg_terminate_backend(7)",
                rows(&["pg_terminate_backend"], &[&["t"]]),
            ),
            ("pg_blocking_pids", rows(&["blocked_pid"], &[])),
            ("FROM pg_stat_user_tables", rows(&["table"], &[&["orders"]])),
            ("FROM pg_stat_user_indexes", rows(&["index"], &[&["i"]])),
            ("FROM pg_stat_statements", rows(&["queryid"], &[&["1"]])),
            ("FROM pg_settings", rows(&["name"], &[&["work_mem"]])),
            (
                "pg_last_xact_replay_timestamp",
                rows(&["in_recovery"], &[&["f"]]),
            ),
            ("FROM pg_stat_replication", rows(&["state"], &[])),
            ("vacuum", command("VACUUM", None)),
            ("update demo.orders", command("UPDATE", Some(1))),
        ]);
        let connection = ConnectionConfig::from_vars(
            |name| match name {
                "PGHOST" => Some("h".into()),
                "PGDATABASE" => Some("d".into()),
                "PGUSER" => Some("u".into()),
                _ => None,
            },
            &NoFiles,
        )
        .unwrap();
        let config = ServerConfig::new(connection.clone(), level);
        PostgresMcpHost::new(
            Some(RunQuery::new(gateway.clone())),
            Some(InspectServer::new(gateway.clone())),
            Some(ChangeServer::new(gateway.clone(), level)),
            Some(DiscoverScope::new(
                gateway.clone(),
                level,
                connection.display(),
            )),
            DiagnoseSetup::new(Ok(config), Some(gateway)),
            level,
        )
    }

    fn unconfigured() -> PostgresMcpHost {
        PostgresMcpHost::new(
            None,
            None,
            None,
            None,
            DiagnoseSetup::new(Err(ConfigError::MissingConnection), None),
            WriteLevel::Off,
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
        let host = host(WriteLevel::All);
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "ok");
        assert_eq!(
            ok(host.call("discover", json!({"schema": "demo"})).await)["server"]["user"],
            "app"
        );
        assert_eq!(
            ok(host
                .call("query", json!({"sql": "select 1", "max_rows": 5}))
                .await)["rows"][0][0],
            "1"
        );
        assert_eq!(
            ok(host
                .call("explain", json!({"sql": "select 1", "format": "text"}))
                .await)["plan"][0],
            "Result"
        );
        assert_eq!(ok(host.call("activity", json!({})).await)["count"], 1);
        assert_eq!(ok(host.call("locks", json!({})).await)["count"], 0);
        assert_eq!(
            ok(host.call("table_stats", json!({"order": "size"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("index_stats", json!({"unused_only": true})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("statements", json!({"order": "calls"})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("settings", json!({"names": ["work_mem"]})).await)["count"],
            1
        );
        assert_eq!(
            ok(host.call("replication", json!({})).await)["role"],
            "primary"
        );
        assert_eq!(
            ok(host
                .call("execute", json!({"sql": "vacuum demo.orders"}))
                .await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call(
                    "execute",
                    json!({"sql": "vacuum demo.orders", "confirm": true})
                )
                .await)["commandTag"],
            "VACUUM"
        );
        assert_eq!(
            ok(host
                .call(
                    "execute",
                    json!({"sql": "update demo.orders set x = 1", "confirm": true})
                )
                .await)["affected"],
            1
        );
        assert_eq!(
            ok(host.call("terminate", json!({"pid": 7})).await)["mode"],
            "preview"
        );
        assert_eq!(
            ok(host
                .call("terminate", json!({"pid": 7, "confirm": true}))
                .await)["signalled"],
            true
        );
        assert_eq!(host.tools().len(), 13);
    }

    #[tokio::test]
    async fn bad_input_fails_before_any_statement() {
        let host = host(WriteLevel::All);
        for (name, args, expected) in [
            ("query", json!({}), "sql is required"),
            ("query", json!({"sql": "delete from t"}), "not a read"),
            (
                "query",
                json!({"sql": "select 1", "max_rows": 0}),
                "max_rows",
            ),
            ("explain", json!({}), "sql is required"),
            (
                "explain",
                json!({"sql": "select 1", "format": "yaml"}),
                "format",
            ),
            ("activity", json!({"limit": 0}), "limit"),
            ("table_stats", json!({"order": "bloat"}), "order"),
            ("settings", json!({}), "names or prefix"),
            ("execute", json!({}), "sql is required"),
            ("execute", json!({"sql": "select 1"}), "is a read"),
            ("terminate", json!({}), "pid is required"),
            ("terminate", json!({"pid": -1}), "pid is required"),
            ("unknown", json!({}), "unknown tool"),
        ] {
            let ToolOutcome::Failed { error, .. } = host.call(name, args).await else {
                panic!("{name}")
            };
            assert!(error.contains(expected), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn the_write_level_hides_or_refuses_the_writes() {
        let off = host(WriteLevel::Off);
        assert_eq!(off.tools().len(), 11);
        let ToolOutcome::Failed { hint, .. } = off
            .call("execute", json!({"sql": "vacuum t", "confirm": true}))
            .await
        else {
            panic!()
        };
        assert!(hint.unwrap().contains("POSTGRES_WRITES"));
        let maintenance = host(WriteLevel::Maintenance);
        assert_eq!(maintenance.tools().len(), 13);
        let ToolOutcome::Failed { error, .. } = maintenance
            .call(
                "execute",
                json!({"sql": "delete from demo.orders", "confirm": true}),
            )
            .await
        else {
            panic!()
        };
        assert!(error.contains("POSTGRES_WRITES=all"));
        let ToolOutcome::Failed { error, .. } = maintenance
            .call("terminate", json!({"pid": 7, "confirm": true}))
            .await
        else {
            panic!()
        };
        assert!(error.contains("POSTGRES_WRITES=all"));
    }

    #[tokio::test]
    async fn without_configuration_only_doctor_answers() {
        let host = unconfigured();
        assert_eq!(ok(host.call("doctor", json!({})).await)["status"], "failed");
        for name in ["discover", "query", "execute"] {
            let ToolOutcome::Failed { hint, .. } =
                host.call(name, json!({"sql": "select 1"})).await
            else {
                panic!("{name}")
            };
            assert!(hint.unwrap().contains("doctor"));
        }
    }

    #[tokio::test]
    async fn the_protocol_publishes_the_catalogue_and_the_instructions() {
        let server = StdioServer::new(Arc::new(host(WriteLevel::Off)));
        let init = server.handle(json!(1), "initialize", json!({})).await;
        assert_eq!(init["result"]["serverInfo"]["name"], "postgres-mcp");
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("POSTGRES_WRITES")
        );
        let list = server.handle(json!(2), "tools/list", json!({})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 11);
        let call = server
            .handle(json!(3), "tools/call", json!({"name": "doctor"}))
            .await;
        assert_eq!(call["result"]["isError"], false);
    }
}
