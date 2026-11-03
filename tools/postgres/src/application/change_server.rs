use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{TIMEOUT_DEFAULT, fixed_read, rows_of, timeout};
use crate::domain::{Fixed, InputError, Kind, Statement, WriteLevel};
use crate::ports::{Outcome, SqlGateway};

const NEXT: &str = "call again with confirm=true to run it";

/// The writes: `execute` and `terminate`. Every one previews unless
/// `confirm` is true; the preview reads (the relation's facts, a plan) and
/// writes nothing. The level set at start decides what may run at all.
pub struct ChangeServer {
    gateway: Arc<dyn SqlGateway>,
    level: WriteLevel,
}

impl ChangeServer {
    pub fn new(gateway: Arc<dyn SqlGateway>, level: WriteLevel) -> Self {
        Self { gateway, level }
    }

    /// A maintenance statement or, at the `all` level, anything but a read.
    /// The preview names the kind and the relation with its size and
    /// statistics; for DML it adds the plan, from an EXPLAIN that is rolled
    /// back. Confirmed, a statement that refuses a transaction block runs
    /// alone; the rest run in one transaction and report the command tag.
    pub async fn execute(&self, sql: &str, timeout_seconds: i64, confirm: bool) -> Result<Value> {
        let statement = Statement::parse(sql)?;
        statement.require_write(self.level)?;
        let timeout_seconds = timeout(timeout_seconds)?;
        let kind = statement.kind().clone();
        let mut answer = json!({
            "mode": "preview",
            "action": "execute",
            "kind": kind.label(),
            "level": kind.level().map(WriteLevel::as_str),
            "statement": statement.text(),
            "outsideTransaction": kind.outside_transaction(),
            "next": NEXT,
        });
        let mut session = self.gateway.connect().await?;
        if let Some(target) = statement.target() {
            answer["target"] = json!(target);
            if let Ok(relation_sql) = Fixed::relation(&target) {
                answer["relation"] =
                    match fixed_read(session.as_mut(), &relation_sql, 5, TIMEOUT_DEFAULT).await {
                        Ok(set) => set.objects().into_iter().next().unwrap_or(Value::Null),
                        Err(error) => json!({"error": format!("{error:#}")}),
                    };
            }
        }
        if let Kind::Dml(_) = kind {
            session.run("BEGIN").await?;
            let plan = session.run(&format!("EXPLAIN {}", statement.text())).await;
            let _ = session.run("ROLLBACK").await;
            answer["plan"] = match plan.and_then(|o| rows_of(o, 200)) {
                Ok(set) => json!(
                    set.rows
                        .iter()
                        .filter_map(|r| r.first().cloned().flatten())
                        .collect::<Vec<_>>()
                ),
                Err(error) => json!({"error": format!("{error:#}")}),
            };
        }
        if !confirm {
            return Ok(answer);
        }
        let outcome = if kind.outside_transaction() {
            session
                .run(&format!("SET statement_timeout = '{timeout_seconds}s'"))
                .await?;
            session.run(statement.text()).await
        } else {
            session.run("BEGIN").await?;
            session
                .run(&format!(
                    "SET LOCAL statement_timeout = '{timeout_seconds}s'"
                ))
                .await?;
            let outcome = session.run(statement.text()).await;
            match &outcome {
                Ok(_) => {
                    session.run("COMMIT").await?;
                }
                Err(_) => {
                    let _ = session.run("ROLLBACK").await;
                }
            }
            outcome
        };
        answer["mode"] = json!("done");
        answer.as_object_mut().map(|o| o.remove("next"));
        match outcome? {
            Outcome::Command { tag, affected } => {
                answer["commandTag"] = json!(tag);
                answer["affected"] = json!(affected);
            }
            Outcome::Rows { columns, rows } => {
                answer["returned"] = rows_of(Outcome::Rows { columns, rows }, 200)?.to_json();
            }
        }
        Ok(answer)
    }

    /// Ends one backend (`pg_terminate_backend`) or only its current query
    /// (`cancel`). The preview shows the backend as `pg_stat_activity` has it.
    pub async fn terminate(&self, pid: i64, cancel_only: bool, confirm: bool) -> Result<Value> {
        if self.level == WriteLevel::Off {
            bail!(InputError::WritesDisabled);
        }
        if self.level < WriteLevel::All {
            bail!(InputError::NeedsAll {
                kind: if cancel_only {
                    "cancel".into()
                } else {
                    "terminate".into()
                }
            });
        }
        let backend_sql = Fixed::backend(pid)?;
        let mut session = self.gateway.connect().await?;
        let backend = fixed_read(session.as_mut(), &backend_sql, 1, TIMEOUT_DEFAULT).await?;
        let Some(backend) = backend.objects().into_iter().next() else {
            bail!("no backend has pid {pid}: activity lists them");
        };
        if backend["backend_type"].as_str() != Some("client backend") {
            bail!(
                "pid {pid} is a {} process, not a client session: it is not ended from here",
                backend["backend_type"].as_str().unwrap_or("background")
            );
        }
        let function = if cancel_only {
            "pg_cancel_backend"
        } else {
            "pg_terminate_backend"
        };
        let mut answer = json!({
            "mode": "preview",
            "action": if cancel_only { "cancel" } else { "terminate" },
            "pid": pid,
            "backend": backend,
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        let outcome = session.run(&format!("SELECT {function}({pid})")).await?;
        let done = rows_of(outcome, 1)?.scalar() == Some("t");
        answer["mode"] = json!("done");
        answer["signalled"] = json!(done);
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, command, rows};
    use super::*;

    fn relation() -> super::super::test_doubles::Answer {
        (
            "FROM pg_class c JOIN pg_namespace n",
            rows(
                &["schema", "relation", "total_size", "dead_rows"],
                &[&["demo", "orders", "12 MB", "120"]],
            ),
        )
    }

    #[tokio::test]
    async fn a_maintenance_statement_previews_the_relation_and_runs_alone() {
        let gateway = FakeGateway::answering(vec![relation(), ("VACUUM", command("VACUUM", None))]);
        let change = ChangeServer::new(gateway.clone(), WriteLevel::Maintenance);
        let preview = change
            .execute("vacuum (analyze) demo.orders", 60, false)
            .await
            .unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["kind"], "VACUUM");
        assert_eq!(preview["target"], "demo.orders");
        assert_eq!(preview["relation"]["dead_rows"], "120");
        assert_eq!(preview["outsideTransaction"], true);
        assert!(!gateway.statements().iter().any(|s| s.starts_with("vacuum")));
        let done = change
            .execute("vacuum (analyze) demo.orders", 60, true)
            .await
            .unwrap();
        assert_eq!(done["mode"], "done");
        assert_eq!(done["commandTag"], "VACUUM");
        let statements = gateway.statements();
        let at = statements
            .iter()
            .position(|s| s == "vacuum (analyze) demo.orders")
            .unwrap();
        assert_eq!(statements[at - 1], "SET statement_timeout = '60s'");
        assert!(
            !statements[at..].iter().any(|s| s == "BEGIN"),
            "VACUUM runs outside a transaction"
        );
    }

    #[tokio::test]
    async fn dml_previews_its_plan_and_commits_only_when_confirmed() {
        let gateway = FakeGateway::answering(vec![
            relation(),
            (
                "EXPLAIN",
                rows(
                    &["QUERY PLAN"],
                    &[&["Update on orders (cost=0.00..1.00 rows=3 width=4)"]],
                ),
            ),
            ("update demo.orders", command("UPDATE", Some(3))),
        ]);
        let change = ChangeServer::new(gateway.clone(), WriteLevel::All);
        let preview = change
            .execute("update demo.orders set x = 1 where id < 4", 30, false)
            .await
            .unwrap();
        assert_eq!(
            preview["plan"][0].as_str().unwrap(),
            "Update on orders (cost=0.00..1.00 rows=3 width=4)"
        );
        assert_eq!(preview["level"], "all");
        let statements = gateway.statements();
        let explain = statements
            .iter()
            .position(|s| s.starts_with("EXPLAIN update"))
            .unwrap();
        assert_eq!(statements[explain - 1], "BEGIN");
        assert_eq!(statements[explain + 1], "ROLLBACK");
        assert!(!statements.iter().any(|s| s.starts_with("update")));
        let done = change
            .execute("update demo.orders set x = 1 where id < 4", 30, true)
            .await
            .unwrap();
        assert_eq!(done["affected"], 3);
        let statements = gateway.statements();
        let run = statements
            .iter()
            .position(|s| s.starts_with("update"))
            .unwrap();
        assert_eq!(statements[run - 1], "SET LOCAL statement_timeout = '30s'");
        assert_eq!(statements[run + 1], "COMMIT");
        gateway.set(vec![
            relation(),
            (
                "update demo.orders",
                Err("permission denied for table orders".into()),
            ),
        ]);
        let error = change
            .execute("update demo.orders set x = 1", 30, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("permission denied"));
        assert_eq!(gateway.statements().last().unwrap(), "ROLLBACK");
        gateway.set(vec![("insert into demo.orders", rows(&["id"], &[&["9"]]))]);
        let returning = change
            .execute(
                "insert into demo.orders (x) values (1) returning id",
                30,
                true,
            )
            .await
            .unwrap();
        assert_eq!(returning["returned"]["rows"][0][0], "9");
    }

    #[tokio::test]
    async fn the_level_and_the_statement_are_checked_before_connecting() {
        let gateway = FakeGateway::answering(vec![]);
        let off = ChangeServer::new(gateway.clone(), WriteLevel::Off);
        assert!(
            off.execute("vacuum t", 30, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("POSTGRES_WRITES")
        );
        assert!(
            off.terminate(7, false, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("POSTGRES_WRITES")
        );
        let maintenance = ChangeServer::new(gateway.clone(), WriteLevel::Maintenance);
        for (sql, expected) in [
            ("update t set x = 1", "POSTGRES_WRITES=all"),
            ("create index i on t (x)", "CONCURRENTLY"),
            ("select 1", "is a read"),
            ("commit", "not run here"),
            ("", "sql is required"),
            ("vacuum t; vacuum u", "one statement"),
        ] {
            let error = maintenance
                .execute(sql, 30, true)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{sql}: {error}");
        }
        assert!(maintenance.execute("vacuum t", 0, true).await.is_err());
        assert!(
            maintenance
                .terminate(7, true, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("POSTGRES_WRITES=all")
        );
        assert_eq!(*gateway.connections.lock().unwrap(), 0);
        assert!(
            ChangeServer::new(FakeGateway::refusing(), WriteLevel::All)
                .execute("vacuum t", 30, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn terminate_previews_the_backend_and_signals_it() {
        let gateway = FakeGateway::answering(vec![
            (
                "WHERE pid = 42",
                rows(
                    &["pid", "state", "query", "backend_type"],
                    &[&["42", "active", "select pg_sleep(100)", "client backend"]],
                ),
            ),
            (
                "WHERE pid = 44",
                rows(&["pid", "backend_type"], &[&["44", "checkpointer"]]),
            ),
            ("WHERE pid = 43", rows(&["pid"], &[])),
            (
                "pg_terminate_backend(42)",
                rows(&["pg_terminate_backend"], &[&["t"]]),
            ),
            (
                "pg_cancel_backend(42)",
                rows(&["pg_cancel_backend"], &[&["f"]]),
            ),
        ]);
        let change = ChangeServer::new(gateway.clone(), WriteLevel::All);
        let preview = change.terminate(42, false, false).await.unwrap();
        assert_eq!(preview["backend"]["query"], "select pg_sleep(100)");
        assert_eq!(preview["action"], "terminate");
        assert!(
            !gateway
                .statements()
                .iter()
                .any(|s| s.contains("pg_terminate_backend("))
        );
        let done = change.terminate(42, false, true).await.unwrap();
        assert_eq!(done["signalled"], true);
        let cancel = change.terminate(42, true, true).await.unwrap();
        assert_eq!(cancel["action"], "cancel");
        assert_eq!(cancel["signalled"], false);
        assert!(
            change
                .terminate(43, false, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("no backend has pid 43")
        );
        assert!(change.terminate(0, false, true).await.is_err());
        let error = change
            .terminate(44, false, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("checkpointer process"), "{error}");
        assert!(
            !gateway
                .statements()
                .iter()
                .any(|s| s.contains("pg_terminate_backend(44)"))
        );
    }
}
