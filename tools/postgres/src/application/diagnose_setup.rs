use std::sync::Arc;

use serde_json::{Value, json};

use super::shared::{TIMEOUT_DEFAULT, fixed_read};
use crate::domain::{ConfigError, Fixed, PasswordSource, ServerConfig};
use crate::ports::SqlGateway;

/// `doctor`: the connection as configured (never the password), where the
/// password comes from, whether the server answers and as whom, the role's
/// powers, `pg_stat_statements`, and the write level. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    gateway: Option<Arc<dyn SqlGateway>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        gateway: Option<Arc<dyn SqlGateway>>,
    ) -> Self {
        Self { config, gateway }
    }

    pub async fn execute(&self) -> Value {
        let mut checks = Vec::new();
        let config = match &self.config {
            Ok(config) => {
                checks.push(check("configuration", "ok", config.summary(), None));
                config
            }
            Err(error) => {
                checks.push(check(
                    "configuration",
                    "failed",
                    error.to_string(),
                    Some(action_for(error)),
                ));
                return report(checks);
            }
        };
        let (status, detail, action) = match config.connection().password_source {
            PasswordSource::Url => ("ok", "from DATABASE_URL", None),
            PasswordSource::Environment => ("ok", "from PGPASSWORD", None),
            PasswordSource::PasswordFile => ("ok", "from the password file", None),
            PasswordSource::None => (
                "ok",
                "none: the server must accept this user without one (trust, peer, or a role on the node)",
                None,
            ),
            PasswordSource::FileTooOpen => (
                "failed",
                "the password file is readable by group or others, so it is ignored, as libpq ignores it",
                Some("chmod 0600 the password file".to_string()),
            ),
        };
        checks.push(check("password", status, detail, action));
        let Some(gateway) = &self.gateway else {
            checks.push(check(
                "gateway",
                "failed",
                "no database client",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        let mut session = match gateway.connect().await {
            Ok(session) => session,
            Err(error) => {
                checks.push(check("connection", "failed", format!("{error:#}"), Some("check PGHOST, PGPORT, the network from this node, the TLS mode and the password".into())));
                return report(checks);
            }
        };
        match fixed_read(session.as_mut(), Fixed::SERVER, 1, TIMEOUT_DEFAULT).await {
            Ok(set) => {
                let row = set.objects().into_iter().next().unwrap_or(Value::Null);
                checks.push(check(
                    "server",
                    "ok",
                    format!(
                        "{} as {} on {}{}",
                        row["version"]
                            .as_str()
                            .unwrap_or("?")
                            .split(' ')
                            .take(2)
                            .collect::<Vec<_>>()
                            .join(" "),
                        row["user"].as_str().unwrap_or("?"),
                        row["database"].as_str().unwrap_or("?"),
                        if row["in_recovery"].as_str() == Some("t") {
                            " (standby, read only)"
                        } else {
                            ""
                        }
                    ),
                    None,
                ));
            }
            Err(error) => {
                checks.push(check("server", "failed", format!("{error:#}"), Some("the connection opened but the first query failed: check the role's CONNECT privilege".into())));
                return report(checks);
            }
        }
        match fixed_read(session.as_mut(), "SELECT rolsuper AS superuser, rolcreatedb AS createdb, rolreplication AS replication, rolbypassrls AS bypassrls FROM pg_roles WHERE rolname = current_user", 1, TIMEOUT_DEFAULT).await {
            Ok(set) => {
                let row = set.objects().into_iter().next().unwrap_or(Value::Null);
                let superuser = row["superuser"].as_str() == Some("t");
                checks.push(check(
                    "role",
                    "ok",
                    if superuser { "superuser: every statement the write level allows will run; prefer a role with less" } else { "not a superuser" },
                    None,
                ));
            }
            Err(error) => checks.push(check("role", "failed", format!("{error:#}"), None)),
        }
        match fixed_read(
            session.as_mut(),
            Fixed::STATEMENTS_COLUMNS,
            5,
            TIMEOUT_DEFAULT,
        )
        .await
        {
            Ok(set) if !set.rows.is_empty() => checks.push(check(
                "pg_stat_statements",
                "ok",
                "installed: statements answers",
                None,
            )),
            Ok(_) => checks.push(check(
                "pg_stat_statements",
                "ok",
                "not installed: statements will say so; the rest works",
                None,
            )),
            Err(error) => checks.push(check(
                "pg_stat_statements",
                "ok",
                format!("could not be checked: {error:#}"),
                None,
            )),
        }
        checks.push(check(
            "writes",
            "ok",
            match config.writes() {
                crate::domain::WriteLevel::Off => "off: only the reads are published".to_string(),
                level => format!(
                    "{}: execute and terminate are published; each previews unless confirm=true",
                    level.as_str()
                ),
            },
            None,
        ));
        report(checks)
    }
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::MissingConnection | ConfigError::InvalidUrl(_) => "set DATABASE_URL (postgres://user@host:5432/db?sslmode=verify-full), or PGHOST, PGDATABASE and PGUSER with PGPASSFILE".into(),
        ConfigError::Invalid(name, _) => format!("fix {name}"),
        ConfigError::InvalidSslMode => "set PGSSLMODE to disable, prefer, require or verify-full".into(),
        ConfigError::InvalidWrites => "set POSTGRES_WRITES to off, maintenance or all, or unset it".into(),
    }
}

fn check(name: &str, status: &str, detail: impl Into<String>, action: Option<String>) -> Value {
    json!({"check": name, "status": status, "detail": detail.into(), "action": action})
}

fn report(checks: Vec<Value>) -> Value {
    let ok = checks.iter().all(|c| c["status"] == "ok");
    json!({"status": if ok { "ok" } else { "failed" }, "checks": checks})
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::test_doubles::{FakeGateway, rows};
    use super::*;
    use crate::domain::{ConnectionConfig, Files, WriteLevel};

    struct NoFiles;

    impl Files for NoFiles {
        fn read(&self, _: &Path) -> Option<(String, u32)> {
            None
        }
    }

    fn config(writes: WriteLevel, password: Option<&str>) -> Result<ServerConfig, ConfigError> {
        let connection = ConnectionConfig::from_vars(
            |name| match name {
                "PGHOST" => Some("db.example".into()),
                "PGDATABASE" => Some("orders".into()),
                "PGUSER" => Some("app".into()),
                "PGPASSWORD" => password.map(str::to_string),
                _ => None,
            },
            &NoFiles,
        )
        .unwrap();
        Ok(ServerConfig::new(connection, writes))
    }

    fn healthy() -> Vec<super::super::test_doubles::Answer> {
        vec![
            (
                "version()",
                rows(
                    &["version", "user", "database", "in_recovery"],
                    &[&["PostgreSQL 16.4 on x86_64", "app", "orders", "f"]],
                ),
            ),
            ("FROM pg_roles", rows(&["superuser"], &[&["f"]])),
            (
                "information_schema.columns",
                rows(&["column_name"], &[&["total_exec_time"]]),
            ),
        ]
    }

    fn statuses(report: &Value) -> Vec<(String, String)> {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["check"].as_str().unwrap().into(),
                    c["status"].as_str().unwrap().into(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn a_working_database_passes_and_shows_no_password() {
        let gateway = FakeGateway::answering(healthy());
        let report = DiagnoseSetup::new(
            config(WriteLevel::Maintenance, Some("s3cret")),
            Some(gateway),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(
            statuses(&report),
            [
                ("configuration", "ok"),
                ("password", "ok"),
                ("server", "ok"),
                ("role", "ok"),
                ("pg_stat_statements", "ok"),
                ("writes", "ok")
            ]
            .map(|(a, b)| (a.into(), b.into()))
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("PostgreSQL 16.4 as app on orders")
        );
        assert!(
            report["checks"][5]["detail"]
                .as_str()
                .unwrap()
                .contains("maintenance")
        );
        assert!(!report.to_string().contains("s3cret"));
        let standby = FakeGateway::answering(vec![
            (
                "version()",
                rows(
                    &["version", "user", "database", "in_recovery"],
                    &[&["PostgreSQL 15.1", "ro", "orders", "t"]],
                ),
            ),
            ("FROM pg_roles", rows(&["superuser"], &[&["t"]])),
            ("information_schema.columns", rows(&["column_name"], &[])),
        ]);
        let report = DiagnoseSetup::new(config(WriteLevel::Off, None), Some(standby))
            .execute()
            .await;
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("standby")
        );
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("superuser:")
        );
        assert!(
            report["checks"][4]["detail"]
                .as_str()
                .unwrap()
                .contains("not installed")
        );
        assert!(
            report["checks"][1]["detail"]
                .as_str()
                .unwrap()
                .contains("none")
        );
    }

    #[tokio::test]
    async fn each_failure_stops_with_its_action() {
        let report = DiagnoseSetup::new(Err(ConfigError::MissingConnection), None)
            .execute()
            .await;
        assert_eq!(
            statuses(&report),
            [("configuration".to_string(), "failed".to_string())]
        );
        assert!(
            report["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("DATABASE_URL")
        );
        let report = DiagnoseSetup::new(config(WriteLevel::Off, None), None)
            .execute()
            .await;
        assert_eq!(statuses(&report)[2].0, "gateway");
        let report =
            DiagnoseSetup::new(config(WriteLevel::Off, None), Some(FakeGateway::refusing()))
                .execute()
                .await;
        assert_eq!(statuses(&report)[2], ("connection".into(), "failed".into()));
        assert!(
            report["checks"][2]["action"]
                .as_str()
                .unwrap()
                .contains("PGHOST")
        );
        let first_query_fails = FakeGateway::answering(vec![(
            "version()",
            Err("permission denied for database".into()),
        )]);
        let report = DiagnoseSetup::new(config(WriteLevel::Off, None), Some(first_query_fails))
            .execute()
            .await;
        assert_eq!(statuses(&report)[2], ("server".into(), "failed".into()));
        let mut answers = healthy();
        answers[1] = ("FROM pg_roles", Err("timeout".into()));
        answers[2] = ("information_schema.columns", Err("timeout".into()));
        let report = DiagnoseSetup::new(
            config(WriteLevel::All, None),
            Some(FakeGateway::answering(answers)),
        )
        .execute()
        .await;
        assert_eq!(statuses(&report)[3], ("role".into(), "failed".into()));
        assert!(
            report["checks"][4]["detail"]
                .as_str()
                .unwrap()
                .contains("could not be checked")
        );
        for error in [
            ConfigError::MissingConnection,
            ConfigError::InvalidUrl("x".into()),
            ConfigError::Invalid("PGPORT", "x".into()),
            ConfigError::InvalidSslMode,
            ConfigError::InvalidWrites,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }

    #[tokio::test]
    async fn an_open_password_file_is_a_failure() {
        struct Open;
        impl Files for Open {
            fn read(&self, _: &Path) -> Option<(String, u32)> {
                Some(("*:*:*:*:p\n".into(), 0o644))
            }
        }
        let connection = ConnectionConfig::from_vars(
            |name| match name {
                "PGHOST" => Some("h".into()),
                "PGDATABASE" => Some("d".into()),
                "PGUSER" => Some("u".into()),
                "PGPASSFILE" => Some("/x/.pgpass".into()),
                _ => None,
            },
            &Open,
        )
        .unwrap();
        let report = DiagnoseSetup::new(
            Ok(ServerConfig::new(connection, WriteLevel::Off)),
            Some(FakeGateway::answering(healthy())),
        )
        .execute()
        .await;
        assert_eq!(report["status"], "failed");
        assert!(
            report["checks"][1]["action"]
                .as_str()
                .unwrap()
                .contains("chmod")
        );
    }
}
