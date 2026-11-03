use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{cmd, key_facts, preview_text, shown};
use crate::domain::{InputError, Reply, ValueFilter, key_name, key_pattern};
use crate::ports::RedisGateway;

pub const KEYS_MAX: i64 = 1_000;
const SCAN_COUNT: usize = 500;
const EXAMINED_MAX: usize = 50_000;
const BIGGEST_SAMPLE_MAX: usize = 20_000;

/// The key reads: `keys` (SCAN, never KEYS), `key`, `biggest`.
pub struct InspectKeys {
    gateway: Arc<dyn RedisGateway>,
}

impl InspectKeys {
    pub fn new(gateway: Arc<dyn RedisGateway>) -> Self {
        Self { gateway }
    }

    /// Keys by pattern, with their facts when `with_values` or when a
    /// value filter needs them. The filter sees the preview (the first 50
    /// elements, 64 KiB of a string), not the whole value.
    pub async fn keys(
        &self,
        pattern: Option<&str>,
        value_contains: Option<&str>,
        ignore_case: bool,
        with_values: bool,
        limit: i64,
    ) -> Result<Value> {
        let filter = ValueFilter::parse(value_contains, ignore_case)?;
        let pattern = key_pattern(pattern, filter.is_some())?;
        if !(1..=KEYS_MAX).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: KEYS_MAX
            });
        }
        let limit = usize::try_from(limit).unwrap_or(1);
        let mut session = self.gateway.connect().await?;
        let read_values = with_values || filter.is_some();
        let mut rows: Vec<Value> = Vec::new();
        let mut cursor = "0".to_string();
        let mut examined = 0usize;
        let mut complete = true;
        'walk: loop {
            let reply = session
                .command(&cmd(&[
                    "SCAN",
                    &cursor,
                    "MATCH",
                    &pattern,
                    "COUNT",
                    &SCAN_COUNT.to_string(),
                ]))
                .await?;
            let items = reply
                .as_array()
                .ok_or(InputError::UnexpectedReply("SCAN"))?;
            cursor = items
                .first()
                .and_then(Reply::to_text)
                .unwrap_or_else(|| "0".into());
            let batch: Vec<String> = items
                .get(1)
                .and_then(Reply::as_array)
                .map(|k| k.iter().filter_map(Reply::to_text).collect())
                .unwrap_or_default();
            for key in batch {
                examined += 1;
                if rows.len() >= limit {
                    complete = false;
                    break 'walk;
                }
                if !read_values {
                    rows.push(json!({"key": key}));
                    continue;
                }
                let Some(facts) = key_facts(session.as_mut(), &key).await? else {
                    continue;
                };
                if filter
                    .as_ref()
                    .is_some_and(|f| !f.matches(&preview_text(&facts)))
                {
                    continue;
                }
                rows.push(shown(facts));
            }
            if cursor == "0" {
                break;
            }
            if examined >= EXAMINED_MAX {
                complete = false;
                break;
            }
        }
        Ok(json!({
            "pattern": pattern,
            "valueFilter": filter.as_ref().map(ValueFilter::describe),
            "valueFilterSees": filter.as_ref().map(|_| "the preview: the first 50 elements of a collection, 64 KiB of a string"),
            "count": rows.len(),
            "examined": examined,
            "complete": complete,
            "keys": rows,
        }))
    }

    /// One key: type, TTL, encoding, memory, preview, and the LFU
    /// frequency or LRU idle time when the policy keeps one.
    pub async fn key(&self, key: &str) -> Result<Value> {
        let key = key_name(key)?;
        let mut session = self.gateway.connect().await?;
        let Some(facts) = key_facts(session.as_mut(), &key).await? else {
            return Ok(json!({"key": key, "exists": false}));
        };
        let mut facts = shown(facts);
        facts["exists"] = json!(true);
        if let Ok(idle) = session.command(&cmd(&["OBJECT", "IDLETIME", &key])).await {
            facts["idleSeconds"] = json!(idle.as_int());
        }
        if let Ok(freq) = session.command(&cmd(&["OBJECT", "FREQ", &key])).await {
            facts["lfuFrequency"] = json!(freq.as_int());
        }
        Ok(facts)
    }

    /// The largest keys of a sample: SCAN with a pattern, MEMORY USAGE per
    /// batch, the top `limit` by bytes.
    pub async fn biggest(&self, pattern: Option<&str>, limit: i64, sample: i64) -> Result<Value> {
        let pattern = pattern
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or("*")
            .to_string();
        if pattern.len() > 512 || pattern.chars().any(char::is_control) || pattern.starts_with('-')
        {
            bail!(InputError::Invalid {
                what: "pattern",
                value: pattern.chars().take(60).collect(),
                rule: "a MATCH glob"
            });
        }
        if !(1..=200).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: 200
            });
        }
        if !(1..=BIGGEST_SAMPLE_MAX as i64).contains(&sample) {
            bail!(InputError::OutOfRange {
                what: "sample",
                min: 1,
                max: BIGGEST_SAMPLE_MAX as i64
            });
        }
        let sample = usize::try_from(sample).unwrap_or(1);
        let mut session = self.gateway.connect().await?;
        let keyspace = session.command(&cmd(&["DBSIZE"])).await?.as_int();
        let mut cursor = "0".to_string();
        let mut sampled = 0usize;
        let mut top: Vec<(i64, String, String, i64)> = Vec::new();
        let mut by_type: std::collections::BTreeMap<String, (usize, i64)> =
            std::collections::BTreeMap::new();
        let mut complete = true;
        loop {
            let reply = session
                .command(&cmd(&[
                    "SCAN",
                    &cursor,
                    "MATCH",
                    &pattern,
                    "COUNT",
                    &SCAN_COUNT.to_string(),
                ]))
                .await?;
            let items = reply
                .as_array()
                .ok_or(InputError::UnexpectedReply("SCAN"))?;
            cursor = items
                .first()
                .and_then(Reply::to_text)
                .unwrap_or_else(|| "0".into());
            let batch: Vec<String> = items
                .get(1)
                .and_then(Reply::as_array)
                .map(|k| k.iter().filter_map(Reply::to_text).collect())
                .unwrap_or_default();
            let available = batch.len();
            let batch: Vec<String> = batch.into_iter().take(sample - sampled).collect();
            let cut_short = batch.len() < available;
            if !batch.is_empty() {
                let mut commands = Vec::with_capacity(batch.len() * 3);
                for key in &batch {
                    commands.push(cmd(&["TYPE", key]));
                    commands.push(cmd(&["TTL", key]));
                    commands.push(cmd(&["MEMORY", "USAGE", key, "SAMPLES", "0"]));
                }
                let replies = session.pipeline(&commands).await?;
                for (i, key) in batch.iter().enumerate() {
                    let kind = replies
                        .get(i * 3)
                        .and_then(Reply::to_text)
                        .unwrap_or_else(|| "?".into());
                    let ttl = replies.get(i * 3 + 1).and_then(Reply::as_int).unwrap_or(-1);
                    let bytes = replies.get(i * 3 + 2).and_then(Reply::as_int).unwrap_or(0);
                    let entry = by_type.entry(kind.clone()).or_insert((0, 0));
                    entry.0 += 1;
                    entry.1 += bytes;
                    top.push((bytes, key.clone(), kind, ttl));
                }
                top.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                top.truncate(usize::try_from(limit).unwrap_or(1));
                sampled += batch.len();
            }
            if cut_short || (sampled >= sample && cursor != "0") {
                complete = false;
                break;
            }
            if cursor == "0" {
                break;
            }
        }
        Ok(json!({
            "pattern": pattern,
            "keyspace": keyspace,
            "sampled": sampled,
            "complete": complete,
            "byType": by_type.iter().map(|(k, (n, b))| json!({"type": k, "keys": n, "bytes": b})).collect::<Vec<_>>(),
            "keys": top.iter().map(|(bytes, key, kind, ttl)| json!({"key": key, "type": kind, "bytes": bytes, "ttlSeconds": if *ttl >= 0 { json!(ttl) } else { Value::Null }})).collect::<Vec<_>>(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, text, texts};
    use super::*;

    fn scan_page(cursor: &str, keys: &[&str]) -> Reply {
        Reply::Array(vec![text(cursor), texts(keys)])
    }

    fn facts(
        key: &'static str,
        kind: &str,
        value: Reply,
    ) -> Vec<super::super::test_doubles::Answer> {
        let mut v: Vec<super::super::test_doubles::Answer> = vec![];
        v.push((
            Box::leak(format!("TYPE {key}").into_boxed_str()),
            Ok(text(kind)),
        ));
        v.push((
            Box::leak(format!("TTL {key}").into_boxed_str()),
            Ok(Reply::Int(60)),
        ));
        v.push((
            Box::leak(format!("OBJECT ENCODING {key}").into_boxed_str()),
            Ok(text("raw")),
        ));
        v.push((
            Box::leak(format!("MEMORY USAGE {key}").into_boxed_str()),
            Ok(Reply::Int(100)),
        ));
        v.push((
            Box::leak(format!("GETRANGE {key}").into_boxed_str()),
            Ok(value),
        ));
        v
    }

    #[tokio::test]
    async fn keys_walk_scan_with_a_value_filter_and_stop_at_the_limit() {
        let mut answers = vec![
            (
                "SCAN 0 MATCH demo:*",
                Ok(scan_page("7", &["demo:a", "demo:b"])),
            ),
            ("SCAN 7 MATCH demo:*", Ok(scan_page("0", &["demo:c"]))),
            ("SCAN 0 MATCH *", Ok(scan_page("0", &["demo:a"]))),
        ];
        answers.extend(facts("demo:a", "string", text("status=PENDING")));
        answers.extend(facts("demo:b", "string", text("status=done")));
        answers.extend(facts("demo:c", "string", text("pending too")));
        let gateway = FakeGateway::answering(answers);
        let keys = InspectKeys::new(gateway.clone());
        let plain = keys
            .keys(Some("demo:*"), None, false, false, 10)
            .await
            .unwrap();
        assert_eq!(plain["count"], 3);
        assert_eq!(plain["complete"], true);
        assert_eq!(plain["examined"], 3);
        assert!(gateway.commands().iter().all(|c| !c.starts_with("KEYS")));
        assert!(
            gateway.commands().iter().all(|c| !c.starts_with("TYPE")),
            "no facts without with_values"
        );
        let filtered = keys
            .keys(Some("demo:*"), Some("pending"), true, false, 10)
            .await
            .unwrap();
        assert_eq!(filtered["count"], 2);
        assert_eq!(filtered["keys"][0]["key"], "demo:a");
        assert_eq!(filtered["keys"][0]["preview"]["value"], "status=PENDING");
        assert!(filtered["keys"][0].get("previewText").is_none());
        assert!(
            filtered["valueFilter"]
                .as_str()
                .unwrap()
                .contains("ignoring case")
        );
        let wide = keys
            .keys(None, Some("pending"), false, false, 10)
            .await
            .unwrap();
        assert_eq!(wide["pattern"], "*");
        let limited = keys
            .keys(Some("demo:*"), None, false, true, 1)
            .await
            .unwrap();
        assert_eq!(limited["count"], 1);
        assert_eq!(limited["complete"], false);
        assert_eq!(limited["keys"][0]["memoryBytes"], 100);
        assert!(
            keys.keys(Some("*"), None, false, false, 10)
                .await
                .unwrap_err()
                .to_string()
                .contains("every key")
        );
        assert!(
            keys.keys(Some("demo:*"), None, false, false, 0)
                .await
                .is_err()
        );
        gateway.set(vec![("SCAN", Ok(text("not an array")))]);
        assert!(
            keys.keys(Some("demo:*"), None, false, false, 10)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn one_key_and_the_biggest_keys() {
        let mut answers = facts("demo:a", "string", text("v"));
        answers.push(("OBJECT IDLETIME demo:a", Ok(Reply::Int(30))));
        answers.push((
            "OBJECT FREQ demo:a",
            Err("ERR An LFU maxmemory policy is not selected".into()),
        ));
        answers.push(("TYPE demo:gone", Ok(text("none"))));
        answers.push(("DBSIZE", Ok(Reply::Int(3))));
        answers.push((
            "SCAN 0 MATCH *",
            Ok(scan_page("0", &["demo:a", "demo:big", "demo:h"])),
        ));
        answers.push(("TYPE demo:big", Ok(text("string"))));
        answers.push(("TTL demo:big", Ok(Reply::Int(-1))));
        answers.push(("MEMORY USAGE demo:big", Ok(Reply::Int(100000))));
        answers.push(("TYPE demo:h", Ok(text("hash"))));
        answers.push(("TTL demo:h", Ok(Reply::Int(5))));
        answers.push(("MEMORY USAGE demo:h", Ok(Reply::Int(500))));
        let gateway = FakeGateway::answering(answers);
        let keys = InspectKeys::new(gateway.clone());
        let one = keys.key("demo:a").await.unwrap();
        assert_eq!(one["exists"], true);
        assert_eq!(one["idleSeconds"], 30);
        assert_eq!(one["lfuFrequency"], Value::Null);
        assert_eq!(keys.key("demo:gone").await.unwrap()["exists"], false);
        assert!(keys.key("").await.is_err());
        let biggest = keys.biggest(None, 2, 100).await.unwrap();
        assert_eq!(biggest["keyspace"], 3);
        assert_eq!(biggest["sampled"], 3);
        assert_eq!(biggest["complete"], true);
        assert_eq!(biggest["keys"][0]["key"], "demo:big");
        assert_eq!(biggest["keys"][1]["key"], "demo:h");
        assert_eq!(biggest["keys"][1]["ttlSeconds"], 5);
        assert_eq!(biggest["byType"].as_array().unwrap().len(), 2);
        let capped = keys.biggest(None, 5, 2).await.unwrap();
        assert_eq!(capped["sampled"], 2);
        assert_eq!(capped["complete"], false);
        assert!(keys.biggest(None, 0, 10).await.is_err());
        assert!(keys.biggest(None, 5, 0).await.is_err());
        assert!(keys.biggest(Some("-x"), 5, 10).await.is_err());
        assert!(
            InspectKeys::new(FakeGateway::refusing())
                .key("k")
                .await
                .is_err()
        );
    }
}
