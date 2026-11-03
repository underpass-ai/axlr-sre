use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{begin_read_only, max_rows, rollback, rows_of, timeout};
use crate::domain::{InputError, Kind, Statement};
use crate::ports::SqlGateway;

const CURSOR: &str = "postgres_mcp_cursor";

/// `query` and `explain`: a read inside a read-only transaction, through a
/// cursor so the server sends one row past the cap and no more.
pub struct RunQuery {
    gateway: Arc<dyn SqlGateway>,
}

impl RunQuery {
    pub fn new(gateway: Arc<dyn SqlGateway>) -> Self {
        Self { gateway }
    }

    pub async fn query(&self, sql: &str, max: i64, timeout_seconds: i64) -> Result<Value> {
        let statement = Statement::parse(sql)?;
        statement.require_read()?;
        let max = max_rows(max)?;
        let timeout_seconds = timeout(timeout_seconds)?;
        let mut session = self.gateway.connect().await?;
        begin_read_only(session.as_mut(), timeout_seconds).await?;
        let outcome = if *statement.kind() == Kind::Select {
            let declared = session
                .run(&format!(
                    "DECLARE {CURSOR} NO SCROLL CURSOR FOR {}",
                    statement.text()
                ))
                .await;
            match declared {
                Ok(_) => {
                    session
                        .run(&format!("FETCH {} FROM {CURSOR}", max + 1))
                        .await
                }
                Err(error) => Err(error),
            }
        } else {
            session.run(statement.text()).await
        };
        rollback(session.as_mut()).await;
        let set = rows_of(outcome?, max)?;
        let mut answer = set.to_json();
        answer["kind"] = json!(statement.kind().label());
        answer["maxRows"] = json!(max);
        Ok(answer)
    }

    /// The plan of a read. `analyze` runs it, inside the transaction that is
    /// rolled back; `format` json gives the plan parsed.
    pub async fn explain(
        &self,
        sql: &str,
        analyze: bool,
        format: &str,
        timeout_seconds: i64,
    ) -> Result<Value> {
        let statement = Statement::parse(sql)?;
        if *statement.kind() != Kind::Select {
            bail!(InputError::NotARead(statement.kind().label()));
        }
        statement.require_no_denied_function()?;
        let format = match format.trim().to_ascii_lowercase().as_str() {
            "text" | "" => "TEXT",
            "json" => "JSON",
            other => bail!(InputError::UnknownChoice {
                what: "format",
                value: other.chars().take(20).collect(),
                allowed: "text, json"
            }),
        };
        let timeout_seconds = timeout(timeout_seconds)?;
        let options = if analyze {
            format!("ANALYZE, BUFFERS, FORMAT {format}")
        } else {
            format!("FORMAT {format}")
        };
        let mut session = self.gateway.connect().await?;
        begin_read_only(session.as_mut(), timeout_seconds).await?;
        let outcome = session
            .run(&format!("EXPLAIN ({options}) {}", statement.text()))
            .await;
        rollback(session.as_mut()).await;
        let set = rows_of(outcome?, 10_000)?;
        let lines: Vec<String> = set
            .rows
            .iter()
            .filter_map(|r| r.first().cloned().flatten())
            .collect();
        let plan = if format == "JSON" {
            serde_json::from_str::<Value>(&lines.join("\n")).unwrap_or_else(|_| json!(lines))
        } else {
            json!(lines)
        };
        Ok(json!({
            "analyzed": analyze,
            "format": format.to_ascii_lowercase(),
            "plan": plan,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, rows};
    use super::*;

    #[tokio::test]
    async fn a_select_goes_through_a_cursor_and_the_extra_row_marks_truncation() {
        let gateway = FakeGateway::answering(vec![(
            "FETCH 3 FROM",
            rows(&["id", "name"], &[&["1", "a"], &["2", "b"], &["3", "c"]]),
        )]);
        let answer = RunQuery::new(gateway.clone())
            .query("select id, name from demo.orders", 2, 10)
            .await
            .unwrap();
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["truncated"], true);
        assert_eq!(answer["kind"], "SELECT");
        assert_eq!(answer["rows"][1][1], "b");
        let statements = gateway.statements();
        assert_eq!(
            statements[0],
            "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY"
        );
        assert_eq!(
            statements[3],
            "DECLARE postgres_mcp_cursor NO SCROLL CURSOR FOR select id, name from demo.orders"
        );
        assert_eq!(statements[4], "FETCH 3 FROM postgres_mcp_cursor");
        assert_eq!(statements[5], "ROLLBACK");
        gateway.set(vec![("work_mem", rows(&["work_mem"], &[&["4MB"]]))]);
        let show = RunQuery::new(gateway.clone())
            .query("show work_mem", 10, 10)
            .await
            .unwrap();
        assert_eq!(show["rows"][0][0], "4MB");
        assert!(!gateway.statements().iter().any(|s| s.contains("DECLARE")));
    }

    #[tokio::test]
    async fn bad_reads_fail_before_connecting_and_server_errors_roll_back() {
        let gateway = FakeGateway::answering(vec![]);
        let query = RunQuery::new(gateway.clone());
        for (sql, max, timeout, expected) in [
            ("", 10, 10, "sql is required"),
            ("update t set x = 1", 10, 10, "not a read"),
            ("select pg_terminate_backend(1)", 10, 10, "not allowed"),
            ("select 1; select 2", 10, 10, "one statement"),
            ("select 1", 0, 10, "max_rows"),
            ("select 1", 10, 0, "timeout_seconds"),
        ] {
            let error = query
                .query(sql, max, timeout)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{sql}: {error}");
        }
        assert_eq!(*gateway.connections.lock().unwrap(), 0);
        gateway.set(vec![(
            "DECLARE",
            Err("relation \"demo.nope\" does not exist".into()),
        )]);
        let error = query
            .query("select * from demo.nope", 10, 10)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not exist"));
        assert_eq!(gateway.statements().last().unwrap(), "ROLLBACK");
        assert!(
            RunQuery::new(FakeGateway::refusing())
                .query("select 1", 10, 10)
                .await
                .unwrap_err()
                .to_string()
                .contains("refused")
        );
    }

    #[tokio::test]
    async fn explain_wraps_the_read_and_parses_json() {
        let gateway = FakeGateway::answering(vec![(
            "EXPLAIN (FORMAT JSON)",
            rows(
                &["QUERY PLAN"],
                &[&["[{\"Plan\": {\"Node Type\": \"Seq Scan\", \"Plan Rows\": 42}}]"]],
            ),
        )]);
        let query = RunQuery::new(gateway.clone());
        let plan = query
            .explain("select * from demo.orders", false, "json", 10)
            .await
            .unwrap();
        assert_eq!(plan["plan"][0]["Plan"]["Plan Rows"], 42);
        assert_eq!(plan["analyzed"], false);
        gateway.set(vec![(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)",
            rows(
                &["QUERY PLAN"],
                &[
                    &["Seq Scan on orders (cost=0..1 rows=42)"],
                    &["  Buffers: shared hit=3"],
                ],
            ),
        )]);
        let text = query
            .explain("select * from demo.orders", true, "text", 10)
            .await
            .unwrap();
        assert_eq!(text["plan"].as_array().unwrap().len(), 2);
        assert!(
            gateway
                .statements()
                .iter()
                .any(|s| s.starts_with("EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) select"))
        );
        assert_eq!(gateway.statements().last().unwrap(), "ROLLBACK");
        assert!(query.explain("vacuum t", false, "text", 10).await.is_err());
        assert!(query.explain("select 1", false, "yaml", 10).await.is_err());
        assert!(
            query
                .explain("select pg_read_file('x')", false, "text", 10)
                .await
                .is_err()
        );
        gateway.set(vec![("EXPLAIN", rows(&["QUERY PLAN"], &[&["not json"]]))]);
        let fallback = query.explain("select 1", false, "json", 10).await.unwrap();
        assert_eq!(fallback["plan"][0], "not json");
    }
}
