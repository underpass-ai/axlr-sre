use std::sync::Arc;

use serde_json::{Value, json};

use super::shared::{cmd, info};
use crate::domain::{ConfigError, ServerConfig};
use crate::ports::RedisGateway;

/// `doctor`: the connection as configured (never the password), whether
/// the server answers and what it is, its role and memory limits, cluster
/// mode, and the write switch. No secret is shown.
pub struct DiagnoseSetup {
    config: Result<ServerConfig, ConfigError>,
    gateway: Option<Arc<dyn RedisGateway>>,
}

impl DiagnoseSetup {
    pub fn new(
        config: Result<ServerConfig, ConfigError>,
        gateway: Option<Arc<dyn RedisGateway>>,
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
        checks.push(check(
            "password",
            "ok",
            if config.connection().password().is_some() {
                "set, never shown"
            } else {
                "none: the server must accept this client without one"
            },
            None,
        ));
        let Some(gateway) = &self.gateway else {
            checks.push(check(
                "gateway",
                "failed",
                "no client",
                Some("rebuild the server; this is a bug".into()),
            ));
            return report(checks);
        };
        let mut session = match gateway.connect().await {
            Ok(session) => session,
            Err(error) => {
                checks.push(check("connection", "failed", format!("{error:#}"), Some("check REDIS_HOST and REDIS_PORT, the network from this node, REDIS_TLS and the password".into())));
                return report(checks);
            }
        };
        if let Err(error) = session.command(&cmd(&["PING"])).await {
            checks.push(check("server", "failed", format!("{error:#}"), Some("the connection opened but PING failed: check the password (NOAUTH) and the ACL of this user".into())));
            return report(checks);
        }
        match info(session.as_mut(), "default").await {
            Ok(sections) => {
                let get = |section: &str, key: &str| {
                    sections
                        .get(section)
                        .and_then(|s| s.get(key).cloned())
                        .unwrap_or_else(|| "?".into())
                };
                let version = if get("server", "valkey_version") != "?" {
                    format!("Valkey {}", get("server", "valkey_version"))
                } else {
                    format!("Redis {}", get("server", "redis_version"))
                };
                checks.push(check(
                    "server",
                    "ok",
                    format!(
                        "{version}, mode {}, uptime {} s",
                        mode(&sections),
                        get("server", "uptime_in_seconds")
                    ),
                    None,
                ));
                let role = get("replication", "role");
                checks.push(check(
                    "role",
                    "ok",
                    format!(
                        "{role}{}",
                        if role == "slave" || role == "replica" {
                            ", read only from here".to_string()
                        } else {
                            format!(
                                ", {} connected replicas",
                                get("replication", "connected_slaves")
                            )
                        }
                    ),
                    None,
                ));
                let max = get("memory", "maxmemory");
                checks.push(check(
                    "memory",
                    "ok",
                    format!(
                        "used {}, maxmemory {}, policy {}",
                        get("memory", "used_memory_human"),
                        if max == "0" {
                            "unlimited".to_string()
                        } else {
                            get("memory", "maxmemory_human")
                        },
                        get("memory", "maxmemory_policy")
                    ),
                    None,
                ));
                if get("cluster", "cluster_enabled") == "1" {
                    checks.push(check("cluster", "ok", "cluster mode: keys, biggest and delete see this node's slots only; a MOVED error names the owner of a key", None));
                }
            }
            Err(error) => checks.push(check("info", "failed", format!("{error:#}"), None)),
        }
        checks.push(check(
            "writes",
            "ok",
            if config.writes() { "on: expire, delete, config_set and client_kill are published; each previews unless confirm=true" } else { "off: only the reads are published" },
            None,
        ));
        report(checks)
    }
}

/// Redis says `redis_mode`, Valkey 8 `server_mode`.
fn mode(
    sections: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
) -> String {
    sections
        .get("server")
        .and_then(|s| {
            s.get("redis_mode")
                .or_else(|| s.get("server_mode"))
                .cloned()
        })
        .unwrap_or_else(|| "?".into())
}

fn action_for(error: &ConfigError) -> String {
    match error {
        ConfigError::MissingConnection | ConfigError::InvalidUrl(_) => "set REDIS_URL (redis://user:password@host:6379/0, rediss:// for TLS) or REDIS_HOST with REDIS_PASSWORD".into(),
        ConfigError::Invalid(name, _) => format!("fix {name}"),
        ConfigError::InvalidTls => "set REDIS_TLS to off, on or insecure".into(),
        ConfigError::InvalidWrites => "set REDIS_WRITES to true or false, or unset it".into(),
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
    use super::super::test_doubles::{FakeGateway, text};
    use super::*;
    use crate::domain::ConnectionConfig;

    fn config(writes: bool, password: Option<&str>) -> Result<ServerConfig, ConfigError> {
        let connection = ConnectionConfig::from_vars(|name| match name {
            "REDIS_HOST" => Some("cache.internal".into()),
            "REDIS_PASSWORD" => password.map(str::to_string),
            _ => None,
        })
        .unwrap();
        Ok(ServerConfig::new(connection, writes))
    }

    const INFO: &str = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\nuptime_in_seconds:100\r\n# Replication\r\nrole:master\r\nconnected_slaves:1\r\n# Memory\r\nused_memory_human:1.00M\r\nmaxmemory:0\r\nmaxmemory_policy:noeviction\r\n# Cluster\r\ncluster_enabled:0\r\n";

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
    async fn a_working_instance_passes_and_shows_no_password() {
        let gateway =
            FakeGateway::answering(vec![("PING", Ok(text("PONG"))), ("INFO", Ok(text(INFO)))]);
        let report = DiagnoseSetup::new(config(true, Some("s3cret")), Some(gateway))
            .execute()
            .await;
        assert_eq!(report["status"], "ok", "{report}");
        assert_eq!(
            statuses(&report)
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            [
                "configuration",
                "password",
                "server",
                "role",
                "memory",
                "writes"
            ]
        );
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("Redis 7.2.4")
        );
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("1 connected replicas")
        );
        assert!(
            report["checks"][4]["detail"]
                .as_str()
                .unwrap()
                .contains("unlimited")
        );
        assert!(!report.to_string().contains("s3cret"));
        let valkey = FakeGateway::answering(vec![
            ("PING", Ok(text("PONG"))),
            (
                "INFO",
                Ok(text(
                    "# Server\r\nvalkey_version:8.1.5\r\nredis_mode:cluster\r\n# Replication\r\nrole:replica\r\n# Memory\r\nmaxmemory:1000\r\nmaxmemory_human:1000B\r\n# Cluster\r\ncluster_enabled:1\r\n",
                )),
            ),
        ]);
        let report = DiagnoseSetup::new(config(false, None), Some(valkey))
            .execute()
            .await;
        assert!(
            report["checks"][2]["detail"]
                .as_str()
                .unwrap()
                .contains("Valkey 8.1.5")
        );
        assert!(
            report["checks"][3]["detail"]
                .as_str()
                .unwrap()
                .contains("read only")
        );
        assert_eq!(report["checks"][5]["check"], "cluster");
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
        assert!(
            report["checks"][0]["action"]
                .as_str()
                .unwrap()
                .contains("REDIS_URL")
        );
        let report = DiagnoseSetup::new(config(false, None), None)
            .execute()
            .await;
        assert_eq!(statuses(&report)[2].0, "gateway");
        let report = DiagnoseSetup::new(config(false, None), Some(FakeGateway::refusing()))
            .execute()
            .await;
        assert_eq!(statuses(&report)[2], ("connection".into(), "failed".into()));
        let noauth =
            FakeGateway::answering(vec![("PING", Err("NOAUTH Authentication required".into()))]);
        let report = DiagnoseSetup::new(config(false, None), Some(noauth))
            .execute()
            .await;
        assert!(
            report["checks"][2]["action"]
                .as_str()
                .unwrap()
                .contains("NOAUTH")
        );
        let no_info = FakeGateway::answering(vec![
            ("PING", Ok(text("PONG"))),
            ("INFO", Err("NOPERM".into())),
        ]);
        let report = DiagnoseSetup::new(config(false, None), Some(no_info))
            .execute()
            .await;
        assert_eq!(statuses(&report)[2], ("info".into(), "failed".into()));
        for error in [
            ConfigError::MissingConnection,
            ConfigError::InvalidUrl("x".into()),
            ConfigError::Invalid("REDIS_PORT", "x".into()),
            ConfigError::InvalidTls,
            ConfigError::InvalidWrites,
        ] {
            assert!(!action_for(&error).is_empty());
        }
    }
}
