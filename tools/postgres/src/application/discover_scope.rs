use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{TIMEOUT_DEFAULT, fixed_read};
use crate::domain::{Fixed, WriteLevel};
use crate::ports::SqlGateway;

const TABLES_SHOWN: i64 = 20;

/// `discover`: the database's size, its schemas, the twenty largest
/// tables, the extensions and the settings that matter, and which tools
/// read and which write. One connection, a handful of catalog reads.
pub struct DiscoverScope {
    gateway: Arc<dyn SqlGateway>,
    level: WriteLevel,
    display: String,
}

impl DiscoverScope {
    pub fn new(gateway: Arc<dyn SqlGateway>, level: WriteLevel, display: String) -> Self {
        Self {
            gateway,
            level,
            display,
        }
    }

    pub async fn execute(&self, schema: Option<&str>) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let server = fixed_read(session.as_mut(), Fixed::SERVER, 1, TIMEOUT_DEFAULT).await?;
        let size = fixed_read(session.as_mut(), Fixed::DATABASE_SIZE, 1, TIMEOUT_DEFAULT).await?;
        let schemas = fixed_read(session.as_mut(), Fixed::SCHEMAS, 200, TIMEOUT_DEFAULT).await?;
        let tables = fixed_read(
            session.as_mut(),
            &Fixed::tables(schema, TABLES_SHOWN)?,
            200,
            TIMEOUT_DEFAULT,
        )
        .await?;
        let extensions =
            fixed_read(session.as_mut(), Fixed::EXTENSIONS, 200, TIMEOUT_DEFAULT).await?;
        let settings =
            fixed_read(session.as_mut(), Fixed::KEY_SETTINGS, 200, TIMEOUT_DEFAULT).await?;
        let server = server.objects().into_iter().next().unwrap_or(Value::Null);
        Ok(json!({
            "connection": self.display,
            "server": {
                "version": server["version"].as_str().unwrap_or("?").split(' ').take(2).collect::<Vec<_>>().join(" "),
                "user": server["user"],
                "database": server["database"],
                "inRecovery": server["in_recovery"].as_str() == Some("t"),
            },
            "size": size.objects().into_iter().next().unwrap_or(Value::Null),
            "schemas": schemas.objects(),
            "largestTables": tables.objects(),
            "extensions": extensions.objects().iter().filter_map(|e| e["name"].as_str().map(|n| format!("{n} {}", e["version"].as_str().unwrap_or("")))).collect::<Vec<_>>(),
            "settings": settings.objects(),
            "reads": ["query", "explain", "activity", "locks", "table_stats", "index_stats", "statements", "settings", "replication"],
            "writes": match self.level {
                WriteLevel::Off => json!([]),
                WriteLevel::Maintenance => json!(["execute (VACUUM, ANALYZE, REINDEX, CREATE INDEX CONCURRENTLY, ALTER)"]),
                WriteLevel::All => json!(["execute (any statement)", "terminate"]),
            },
            "writesHint": match self.level {
                WriteLevel::Off => "register with POSTGRES_WRITES=maintenance or all to publish execute",
                _ => "each write previews unless confirm=true",
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, rows};
    use super::*;

    #[tokio::test]
    async fn the_scope_describes_the_database() {
        let gateway = FakeGateway::answering(vec![
            (
                "version()",
                rows(
                    &["version", "user", "database", "in_recovery"],
                    &[&["PostgreSQL 16.4 (Debian)", "app", "orders", "f"]],
                ),
            ),
            (
                "pg_database_size",
                rows(&["size", "bytes"], &[&["42 MB", "44040192"]]),
            ),
            (
                "FROM pg_namespace n LEFT JOIN",
                rows(&["schema", "tables"], &[&["demo", "3"]]),
            ),
            (
                "ORDER BY pg_total_relation_size(c.oid) DESC",
                rows(
                    &["schema", "table", "total_size"],
                    &[&["demo", "orders", "30 MB"]],
                ),
            ),
            (
                "FROM pg_extension",
                rows(&["name", "version"], &[&["pg_stat_statements", "1.10"]]),
            ),
            (
                "FROM pg_settings",
                rows(&["name", "setting"], &[&["work_mem", "4096"]]),
            ),
        ]);
        let discover = DiscoverScope::new(
            gateway.clone(),
            WriteLevel::Maintenance,
            "app@db.example:5432/orders (sslmode prefer)".into(),
        );
        let answer = discover.execute(Some("demo")).await.unwrap();
        assert_eq!(answer["server"]["version"], "PostgreSQL 16.4");
        assert_eq!(answer["size"]["size"], "42 MB");
        assert_eq!(answer["largestTables"][0]["table"], "orders");
        assert_eq!(answer["extensions"][0], "pg_stat_statements 1.10");
        assert_eq!(answer["writes"].as_array().unwrap().len(), 1);
        assert!(
            gateway
                .statements()
                .iter()
                .any(|s| s.contains("n.nspname = 'demo'"))
        );
        let off = DiscoverScope::new(gateway.clone(), WriteLevel::Off, "x".into())
            .execute(None)
            .await
            .unwrap();
        assert_eq!(off["writes"], json!([]));
        let all = DiscoverScope::new(gateway.clone(), WriteLevel::All, "x".into())
            .execute(None)
            .await
            .unwrap();
        assert_eq!(all["writes"].as_array().unwrap().len(), 2);
        assert!(discover.execute(Some("bad schema")).await.is_err());
        assert!(
            DiscoverScope::new(FakeGateway::refusing(), WriteLevel::Off, "x".into())
                .execute(None)
                .await
                .is_err()
        );
    }
}
