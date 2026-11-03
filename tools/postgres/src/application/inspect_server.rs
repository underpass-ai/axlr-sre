use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{TIMEOUT_DEFAULT, fixed_read};
use crate::domain::Fixed;
use crate::ports::SqlGateway;

const FIXED_ROWS: usize = 200;

/// The fixed DBA reads: `activity`, `locks`, `table_stats`, `index_stats`,
/// `statements`, `settings`, `replication`. Each is one read-only
/// transaction over the catalog views.
pub struct InspectServer {
    gateway: Arc<dyn SqlGateway>,
}

impl InspectServer {
    pub fn new(gateway: Arc<dyn SqlGateway>) -> Self {
        Self { gateway }
    }

    async fn fixed(&self, sql: &str) -> Result<Vec<Value>> {
        let mut session = self.gateway.connect().await?;
        Ok(
            fixed_read(session.as_mut(), sql, FIXED_ROWS, TIMEOUT_DEFAULT)
                .await?
                .objects(),
        )
    }

    pub async fn activity(&self, include_idle: bool, limit: i64) -> Result<Value> {
        let sessions = self.fixed(&Fixed::activity(include_idle, limit)?).await?;
        let waiting = sessions
            .iter()
            .filter(|s| !s["wait_event"].is_null())
            .count();
        Ok(
            json!({"count": sessions.len(), "waiting": waiting, "includeIdle": include_idle, "sessions": sessions}),
        )
    }

    pub async fn locks(&self, limit: i64) -> Result<Value> {
        let blocked = self.fixed(&Fixed::locks(limit)?).await?;
        Ok(
            json!({"count": blocked.len(), "blocked": blocked, "hint": if blocked.is_empty() { "nobody waits on a lock" } else { "blocking_pid holds what blocked_pid waits for; terminate (POSTGRES_WRITES=all) ends a backend" }}),
        )
    }

    pub async fn table_stats(
        &self,
        schema: Option<&str>,
        order: &str,
        limit: i64,
    ) -> Result<Value> {
        let tables = self
            .fixed(&Fixed::table_stats(schema, order, limit)?)
            .await?;
        Ok(json!({"count": tables.len(), "order": order, "schema": schema, "tables": tables}))
    }

    pub async fn index_stats(
        &self,
        schema: Option<&str>,
        unused_only: bool,
        limit: i64,
    ) -> Result<Value> {
        let indexes = self
            .fixed(&Fixed::index_stats(schema, unused_only, limit)?)
            .await?;
        let invalid = indexes
            .iter()
            .filter(|i| i["valid"].as_str() == Some("f"))
            .count();
        Ok(
            json!({"count": indexes.len(), "invalid": invalid, "unusedOnly": unused_only, "schema": schema, "indexes": indexes}),
        )
    }

    /// `pg_stat_statements`, naming its time columns as this version does.
    pub async fn statements(&self, order: &str, limit: i64) -> Result<Value> {
        let columns = self.fixed(Fixed::STATEMENTS_COLUMNS).await?;
        let names: Vec<&str> = columns
            .iter()
            .filter_map(|c| c["column_name"].as_str())
            .collect();
        let (total, mean) = if names.contains(&"total_exec_time") {
            ("total_exec_time", "mean_exec_time")
        } else if names.contains(&"total_time") {
            ("total_time", "mean_time")
        } else {
            bail!(
                "pg_stat_statements is not installed in this database: CREATE EXTENSION pg_stat_statements, with the module in shared_preload_libraries"
            )
        };
        let statements = self
            .fixed(&Fixed::statements(order, limit, total, mean)?)
            .await?;
        Ok(json!({"count": statements.len(), "order": order, "statements": statements}))
    }

    pub async fn settings(&self, names: &[String], prefix: Option<&str>) -> Result<Value> {
        let settings = self.fixed(&Fixed::settings(names, prefix)?).await?;
        Ok(json!({"count": settings.len(), "settings": settings}))
    }

    /// Standbys from a primary, or the replay delay on a standby.
    pub async fn replication(&self) -> Result<Value> {
        let recovery = self.fixed(Fixed::RECOVERY).await?;
        let standby = recovery
            .first()
            .is_some_and(|r| r["in_recovery"].as_str() == Some("t"));
        let standbys = if standby {
            Vec::new()
        } else {
            self.fixed(Fixed::REPLICATION).await?
        };
        Ok(json!({
            "role": if standby { "standby" } else { "primary" },
            "recovery": recovery.first().cloned().unwrap_or(Value::Null),
            "standbys": standbys,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, rows};
    use super::*;

    #[tokio::test]
    async fn every_fixed_read_answers_from_its_view() {
        let gateway = FakeGateway::answering(vec![
            (
                "backend_type = 'client backend'",
                rows(
                    &["pid", "state", "wait_event"],
                    &[&["7", "active", "Lock"], &["8", "active", "NULL"]],
                ),
            ),
            (
                "pg_blocking_pids",
                rows(&["blocked_pid", "blocking_pid"], &[&["8", "7"]]),
            ),
            (
                "FROM pg_stat_user_tables",
                rows(&["table", "dead_rows"], &[&["orders", "120"]]),
            ),
            (
                "FROM pg_stat_user_indexes",
                rows(&["index", "valid"], &[&["idx_a", "f"], &["idx_b", "t"]]),
            ),
            (
                "information_schema.columns",
                rows(
                    &["column_name"],
                    &[&["total_exec_time"], &["mean_exec_time"]],
                ),
            ),
            (
                "FROM pg_stat_statements",
                rows(&["queryid", "calls"], &[&["1", "10"]]),
            ),
            (
                "FROM pg_settings",
                rows(&["name", "setting"], &[&["work_mem", "4MB"]]),
            ),
            (
                "pg_last_xact_replay_timestamp",
                rows(&["in_recovery", "replay_delay_seconds"], &[&["f", "NULL"]]),
            ),
            (
                "FROM pg_stat_replication",
                rows(
                    &["application_name", "state"],
                    &[&["standby-1", "streaming"]],
                ),
            ),
        ]);
        let inspect = InspectServer::new(gateway.clone());
        let activity = inspect.activity(false, 50).await.unwrap();
        assert_eq!(activity["count"], 2);
        assert_eq!(activity["waiting"], 1);
        let locks = inspect.locks(20).await.unwrap();
        assert_eq!(locks["count"], 1);
        assert!(locks["hint"].as_str().unwrap().contains("terminate"));
        assert_eq!(
            inspect.table_stats(Some("demo"), "dead", 20).await.unwrap()["tables"][0]["dead_rows"],
            "120"
        );
        let indexes = inspect.index_stats(None, true, 20).await.unwrap();
        assert_eq!(indexes["invalid"], 1);
        let statements = inspect.statements("total", 20).await.unwrap();
        assert_eq!(statements["count"], 1);
        assert!(
            gateway
                .statements()
                .iter()
                .any(|s| s.contains("total_exec_time DESC"))
        );
        assert_eq!(
            inspect.settings(&["work_mem".into()], None).await.unwrap()["settings"][0]["setting"],
            "4MB"
        );
        let replication = inspect.replication().await.unwrap();
        assert_eq!(replication["role"], "primary");
        assert_eq!(replication["standbys"][0]["state"], "streaming");
        assert!(gateway.statements().iter().all(|s| !s.contains("UPDATE")));
    }

    #[tokio::test]
    async fn an_old_version_a_missing_extension_and_a_standby_are_handled() {
        let gateway = FakeGateway::answering(vec![
            (
                "information_schema.columns",
                rows(&["column_name"], &[&["total_time"], &["mean_time"]]),
            ),
            ("FROM pg_stat_statements", rows(&["queryid"], &[&["1"]])),
            (
                "pg_last_xact_replay_timestamp",
                rows(&["in_recovery", "replay_delay_seconds"], &[&["t", "12"]]),
            ),
            ("pg_blocking_pids", rows(&["blocked_pid"], &[])),
        ]);
        let inspect = InspectServer::new(gateway.clone());
        inspect.statements("mean", 5).await.unwrap();
        assert!(
            gateway
                .statements()
                .iter()
                .any(|s| s.contains("mean_time DESC"))
        );
        let replication = inspect.replication().await.unwrap();
        assert_eq!(replication["role"], "standby");
        assert_eq!(replication["recovery"]["replay_delay_seconds"], "12");
        assert!(
            !gateway
                .statements()
                .iter()
                .any(|s| s.contains("pg_stat_replication"))
        );
        assert!(
            inspect.locks(5).await.unwrap()["hint"]
                .as_str()
                .unwrap()
                .contains("nobody")
        );
        gateway.set(vec![(
            "information_schema.columns",
            rows(&["column_name"], &[]),
        )]);
        let error = inspect
            .statements("total", 5)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("not installed"), "{error}");
        assert!(inspect.statements("fastest", 5).await.is_err());
        assert!(inspect.table_stats(None, "bloat", 5).await.is_err());
        assert!(inspect.settings(&[], None).await.is_err());
        assert!(inspect.activity(false, 0).await.is_err());
        assert!(
            InspectServer::new(FakeGateway::refusing())
                .activity(false, 5)
                .await
                .is_err()
        );
    }
}
