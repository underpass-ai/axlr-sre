use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{cmd, info};
use crate::domain::{
    InputError, Reply, client_list, cluster_nodes, latency_latest, pairs_to_map, slowlog,
};
use crate::ports::RedisGateway;

const SECTIONS: [&str; 13] = [
    "server",
    "clients",
    "memory",
    "persistence",
    "stats",
    "replication",
    "cpu",
    "errorstats",
    "cluster",
    "keyspace",
    "commandstats",
    "latencystats",
    "default",
];
const SLOWLOG_MAX: i64 = 128;

/// The instance-wide reads: `info`, `slowlog`, `clients`, `latency`,
/// `memory`, `config_get`, `cluster`.
pub struct InspectInstance {
    gateway: Arc<dyn RedisGateway>,
}

impl InspectInstance {
    pub fn new(gateway: Arc<dyn RedisGateway>) -> Self {
        Self { gateway }
    }

    pub async fn info(&self, section: &str) -> Result<Value> {
        let section = section.trim().to_ascii_lowercase();
        let section = if section.is_empty() {
            "default".to_string()
        } else {
            section
        };
        if !SECTIONS.contains(&section.as_str()) && section != "all" {
            bail!(InputError::UnknownChoice {
                what: "section",
                value: section.chars().take(20).collect(),
                allowed: "server, clients, memory, persistence, stats, replication, cpu, errorstats, cluster, keyspace, commandstats, latencystats, default, all"
            });
        }
        let mut session = self.gateway.connect().await?;
        let sections = info(session.as_mut(), &section).await?;
        Ok(json!({"section": section, "sections": sections}))
    }

    pub async fn slowlog(&self, count: i64) -> Result<Value> {
        if !(1..=SLOWLOG_MAX).contains(&count) {
            bail!(InputError::OutOfRange {
                what: "count",
                min: 1,
                max: SLOWLOG_MAX
            });
        }
        let mut session = self.gateway.connect().await?;
        let replies = session
            .pipeline(&[
                cmd(&["SLOWLOG", "GET", &count.to_string()]),
                cmd(&["SLOWLOG", "LEN"]),
                cmd(&["CONFIG", "GET", "slowlog-log-slower-than"]),
            ])
            .await?;
        let entries = replies.first().map(slowlog).unwrap_or_default();
        Ok(json!({
            "count": entries.len(),
            "total": replies.get(1).and_then(Reply::as_int),
            "thresholdMicroseconds": replies.get(2).map(pairs_to_map).and_then(|m| m.get("slowlog-log-slower-than").cloned()),
            "entries": entries,
        }))
    }

    pub async fn clients(&self) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let reply = session.command(&cmd(&["CLIENT", "LIST"])).await?;
        let clients = client_list(&reply.to_text().unwrap_or_default());
        let blocked = clients
            .iter()
            .filter(|c| c.get("flags").is_some_and(|f| f.contains('b')))
            .count();
        let replicas = clients
            .iter()
            .filter(|c| c.get("flags").is_some_and(|f| f.contains('S')))
            .count();
        Ok(
            json!({"count": clients.len(), "blocked": blocked, "replicaLinks": replicas, "clients": clients}),
        )
    }

    pub async fn latency(&self) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let replies = session
            .pipeline(&[
                cmd(&["LATENCY", "LATEST"]),
                cmd(&["CONFIG", "GET", "latency-monitor-threshold"]),
            ])
            .await?;
        let events = replies.first().map(latency_latest).unwrap_or_default();
        let threshold = replies
            .get(1)
            .map(pairs_to_map)
            .and_then(|m| m.get("latency-monitor-threshold").cloned());
        Ok(json!({
            "count": events.len(),
            "thresholdMs": threshold,
            "hint": if events.is_empty() { "no event above latency-monitor-threshold; 0 means the monitor is off" } else { "latestMs and maxMs per event since the monitor started" },
            "events": events,
        }))
    }

    /// `MEMORY DOCTOR`, `MEMORY STATS` and the memory section of INFO.
    pub async fn memory(&self) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let replies = session
            .pipeline(&[cmd(&["MEMORY", "DOCTOR"]), cmd(&["MEMORY", "STATS"])])
            .await?;
        let sections = info(session.as_mut(), "memory").await?;
        let memory = sections.get("memory").cloned().unwrap_or_default();
        let used: Option<f64> = memory.get("used_memory").and_then(|v| v.parse().ok());
        let max: Option<f64> = memory
            .get("maxmemory")
            .and_then(|v| v.parse().ok())
            .filter(|m| *m > 0.0);
        Ok(json!({
            "doctor": replies.first().and_then(Reply::to_text),
            "usedHuman": memory.get("used_memory_human"),
            "usedBytes": memory.get("used_memory"),
            "peakHuman": memory.get("used_memory_peak_human"),
            "maxmemoryHuman": memory.get("maxmemory_human"),
            "maxmemoryPolicy": memory.get("maxmemory_policy"),
            "usedPercentOfMax": match (used, max) { (Some(u), Some(m)) => json!((u / m * 1000.0).round() / 10.0), _ => Value::Null },
            "fragmentationRatio": memory.get("mem_fragmentation_ratio"),
            "stats": replies.get(1).map(pairs_to_map),
        }))
    }

    pub async fn config_get(&self, pattern: &str) -> Result<Value> {
        let pattern = pattern.trim();
        if pattern.is_empty()
            || pattern.len() > 128
            || pattern.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            bail!(InputError::Invalid {
                what: "pattern",
                value: pattern.chars().take(60).collect(),
                rule: "a parameter name or glob such as maxmemory* or slowlog-*"
            });
        }
        let mut session = self.gateway.connect().await?;
        let reply = session.command(&cmd(&["CONFIG", "GET", pattern])).await?;
        let parameters = pairs_to_map(&reply);
        Ok(json!({"pattern": pattern, "count": parameters.len(), "parameters": parameters}))
    }

    /// `CLUSTER INFO` and `CLUSTER NODES`; a server without cluster mode says so.
    pub async fn cluster(&self) -> Result<Value> {
        let mut session = self.gateway.connect().await?;
        let enabled = info(session.as_mut(), "cluster")
            .await?
            .get("cluster")
            .and_then(|c| c.get("cluster_enabled").cloned())
            .unwrap_or_else(|| "0".into());
        if enabled != "1" {
            return Ok(
                json!({"enabled": false, "hint": "cluster mode is off: one instance, every key here"}),
            );
        }
        let replies = session
            .pipeline(&[cmd(&["CLUSTER", "INFO"]), cmd(&["CLUSTER", "NODES"])])
            .await?;
        let info_text = replies.first().and_then(Reply::to_text).unwrap_or_default();
        let info_map: std::collections::BTreeMap<String, String> = info_text
            .lines()
            .filter_map(|l| l.trim().split_once(':'))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let nodes = replies
            .get(1)
            .and_then(Reply::to_text)
            .map(|t| cluster_nodes(&t))
            .unwrap_or_default();
        Ok(
            json!({"enabled": true, "info": info_map, "nodeCount": nodes.len(), "nodes": nodes, "hint": "keys, biggest and delete see this node's slots only; a MOVED error names the owner of a key"}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, text, texts};
    use super::*;

    #[tokio::test]
    async fn every_instance_read_answers() {
        let gateway = FakeGateway::answering(vec![
            (
                "INFO memory",
                Ok(text(
                    "# Memory\r\nused_memory:5000\r\nused_memory_human:4.88K\r\nmaxmemory:10000\r\nmaxmemory_policy:allkeys-lru\r\nmem_fragmentation_ratio:1.2\r\n",
                )),
            ),
            (
                "INFO cluster",
                Ok(text("# Cluster\r\ncluster_enabled:0\r\n")),
            ),
            ("INFO server", Ok(text("# Server\r\nredis_version:7.2\r\n"))),
            (
                "SLOWLOG GET",
                Ok(Reply::Array(vec![Reply::Array(vec![
                    Reply::Int(1),
                    Reply::Int(2),
                    Reply::Int(300),
                    texts(&["GET", "k"]),
                    text("127.0.0.1:1"),
                    text(""),
                ])])),
            ),
            ("SLOWLOG LEN", Ok(Reply::Int(9))),
            (
                "CONFIG GET slowlog-log-slower-than",
                Ok(texts(&["slowlog-log-slower-than", "10000"])),
            ),
            (
                "CONFIG GET latency-monitor-threshold",
                Ok(texts(&["latency-monitor-threshold", "0"])),
            ),
            (
                "CONFIG GET maxmemory*",
                Ok(texts(&["maxmemory", "0", "maxmemory-policy", "noeviction"])),
            ),
            (
                "CLIENT LIST",
                Ok(text(
                    "id=1 flags=N cmd=client\nid=2 flags=b cmd=blpop\nid=3 flags=S cmd=replconf\n",
                )),
            ),
            ("LATENCY LATEST", Ok(Reply::Array(vec![]))),
            ("MEMORY DOCTOR", Ok(text("Hi Sam, no memory issues"))),
            ("MEMORY STATS", Ok(texts(&["peak.allocated", "6000"]))),
        ]);
        let inspect = InspectInstance::new(gateway.clone());
        assert_eq!(
            inspect.info("server").await.unwrap()["sections"]["server"]["redis_version"],
            "7.2"
        );
        assert_eq!(inspect.info("").await.unwrap()["section"], "default");
        assert!(inspect.info("secrets").await.is_err());
        let slow = inspect.slowlog(10).await.unwrap();
        assert_eq!(slow["count"], 1);
        assert_eq!(slow["total"], 9);
        assert_eq!(slow["thresholdMicroseconds"], "10000");
        assert_eq!(slow["entries"][0]["command"], "GET k");
        assert!(inspect.slowlog(0).await.is_err());
        let clients = inspect.clients().await.unwrap();
        assert_eq!(clients["count"], 3);
        assert_eq!(clients["blocked"], 1);
        assert_eq!(clients["replicaLinks"], 1);
        let latency = inspect.latency().await.unwrap();
        assert_eq!(latency["count"], 0);
        assert!(latency["hint"].as_str().unwrap().contains("off"));
        let memory = inspect.memory().await.unwrap();
        assert_eq!(memory["usedPercentOfMax"], 50.0);
        assert_eq!(memory["maxmemoryPolicy"], "allkeys-lru");
        assert_eq!(memory["stats"]["peak.allocated"], "6000");
        assert!(
            memory["doctor"]
                .as_str()
                .unwrap()
                .contains("no memory issues")
        );
        let config = inspect.config_get("maxmemory*").await.unwrap();
        assert_eq!(config["count"], 2);
        assert!(inspect.config_get("a b").await.is_err());
        let cluster = inspect.cluster().await.unwrap();
        assert_eq!(cluster["enabled"], false);
        gateway.set(vec![
            (
                "INFO cluster",
                Ok(text("# Cluster\r\ncluster_enabled:1\r\n")),
            ),
            (
                "CLUSTER INFO",
                Ok(text("cluster_state:ok\r\ncluster_slots_assigned:16384\r\n")),
            ),
            (
                "CLUSTER NODES",
                Ok(text(
                    "e7d1 127.0.0.1:30001@40001 myself,master - 0 0 1 connected 0-16383\n",
                )),
            ),
        ]);
        let cluster = inspect.cluster().await.unwrap();
        assert_eq!(cluster["enabled"], true);
        assert_eq!(cluster["info"]["cluster_state"], "ok");
        assert_eq!(cluster["nodeCount"], 1);
        assert!(
            InspectInstance::new(FakeGateway::refusing())
                .clients()
                .await
                .is_err()
        );
        gateway.set(vec![(
            "INFO memory",
            Ok(text("# Memory\r\nused_memory:5\r\n")),
        )]);
        assert_eq!(
            inspect.memory().await.unwrap()["usedPercentOfMax"],
            Value::Null
        );
    }
}
