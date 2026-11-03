use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{cmd, info};
use crate::ports::RedisGateway;

/// `discover`: the instance in one read of INFO: version, role, memory,
/// clients, keyspace per database, persistence; and what reads and what
/// writes.
pub struct DiscoverScope {
    gateway: Arc<dyn RedisGateway>,
    writes: bool,
    display: String,
}

impl DiscoverScope {
    pub fn new(gateway: Arc<dyn RedisGateway>, writes: bool, display: String) -> Self {
        Self {
            gateway,
            writes,
            display,
        }
    }

    pub async fn execute(&self) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let sections = info(session.as_mut(), "default").await?;
        let dbsize = session.command(&cmd(&["DBSIZE"])).await?.as_int();
        let get =
            |section: &str, key: &str| sections.get(section).and_then(|s| s.get(key).cloned());
        let keyspace: Vec<Value> = sections
            .get("keyspace")
            .map(|db| {
                db.iter()
                    .map(|(name, value)| json!({"db": name, "stats": value}))
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({
            "connection": self.display,
            "server": {
                "version": get("server", "valkey_version").map(|v| format!("Valkey {v}")).or_else(|| get("server", "redis_version").map(|v| format!("Redis {v}"))),
                "mode": get("server", "redis_mode").or_else(|| get("server", "server_mode")),
                "uptimeSeconds": get("server", "uptime_in_seconds"),
            },
            "role": get("replication", "role"),
            "connectedReplicas": get("replication", "connected_slaves"),
            "memory": {
                "used": get("memory", "used_memory_human"),
                "peak": get("memory", "used_memory_peak_human"),
                "maxmemory": get("memory", "maxmemory_human"),
                "policy": get("memory", "maxmemory_policy"),
                "fragmentationRatio": get("memory", "mem_fragmentation_ratio"),
            },
            "clients": {"connected": get("clients", "connected_clients"), "blocked": get("clients", "blocked_clients")},
            "stats": {"evictedKeys": get("stats", "evicted_keys"), "expiredKeys": get("stats", "expired_keys"), "rejectedConnections": get("stats", "rejected_connections"), "opsPerSecond": get("stats", "instantaneous_ops_per_sec")},
            "persistence": {"rdbLastStatus": get("persistence", "rdb_last_bgsave_status"), "aof": get("persistence", "aof_enabled")},
            "dbsize": dbsize,
            "keyspace": keyspace,
            "reads": ["info", "slowlog", "clients", "latency", "memory", "config_get", "cluster", "keys", "key", "biggest"],
            "writes": if self.writes { json!(["expire", "delete", "config_set", "client_kill"]) } else { json!([]) },
            "writesHint": if self.writes { "each write previews unless confirm=true; FLUSH, SHUTDOWN, DEBUG and data writes have no tool" } else { "register with REDIS_WRITES=true to publish expire, delete, config_set and client_kill" },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, text};
    use super::*;
    use crate::domain::Reply;

    #[tokio::test]
    async fn the_scope_describes_the_instance() {
        let gateway = FakeGateway::answering(vec![
            (
                "INFO",
                Ok(text(
                    "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n# Clients\r\nconnected_clients:3\r\n# Memory\r\nused_memory_human:1.00M\r\nmaxmemory_policy:allkeys-lru\r\n# Stats\r\nevicted_keys:12\r\n# Replication\r\nrole:master\r\n# Keyspace\r\ndb0:keys=42,expires=3,avg_ttl=1000\r\n",
                )),
            ),
            ("DBSIZE", Ok(Reply::Int(42))),
        ]);
        let answer = DiscoverScope::new(gateway.clone(), true, "cache:6379/0 (tls off)".into())
            .execute()
            .await
            .unwrap();
        assert_eq!(answer["server"]["version"], "Redis 7.2.4");
        assert_eq!(answer["memory"]["policy"], "allkeys-lru");
        assert_eq!(answer["stats"]["evictedKeys"], "12");
        assert_eq!(answer["dbsize"], 42);
        assert_eq!(answer["keyspace"][0]["db"], "db0");
        assert_eq!(answer["writes"].as_array().unwrap().len(), 4);
        let off = DiscoverScope::new(gateway, false, "x".into())
            .execute()
            .await
            .unwrap();
        assert_eq!(off["writes"], json!([]));
        assert!(
            DiscoverScope::new(FakeGateway::refusing(), false, "x".into())
                .execute()
                .await
                .is_err()
        );
    }
}
