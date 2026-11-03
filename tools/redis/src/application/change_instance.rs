use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::shared::{cmd, key_facts, preview_text, shown};
use crate::domain::{
    InputError, Reply, ValueFilter, client_list, config_set_check, key_name, key_pattern,
    pairs_to_map,
};
use crate::ports::{RedisGateway, RedisSession};

const NEXT: &str = "call again with confirm=true to run it";
const DELETE_MAX: usize = 1_000;
const EXPIRE_MAX: i64 = 365 * 86_400;
const SCAN_COUNT: &str = "500";
const EXAMINED_MAX: usize = 50_000;

/// The writes: `expire`, `delete`, `config_set`, `client_kill`. Every one
/// previews unless `confirm` is true; the preview reads and writes
/// nothing. The writes must be on, else nothing runs. FLUSH, SHUTDOWN,
/// DEBUG and data writes have no tool.
pub struct ChangeInstance {
    gateway: Arc<dyn RedisGateway>,
    writes: bool,
}

impl ChangeInstance {
    pub fn new(gateway: Arc<dyn RedisGateway>, writes: bool) -> Self {
        Self { gateway, writes }
    }

    fn require_writes(&self) -> Result<()> {
        if self.writes {
            Ok(())
        } else {
            bail!(InputError::WritesDisabled)
        }
    }

    /// A TTL on one existing key.
    pub async fn expire(&self, key: &str, seconds: i64, confirm: bool) -> Result<Value> {
        self.require_writes()?;
        let key = key_name(key)?;
        if !(1..=EXPIRE_MAX).contains(&seconds) {
            bail!(InputError::OutOfRange {
                what: "seconds",
                min: 1,
                max: EXPIRE_MAX
            });
        }
        let mut session = self.gateway.connect().await?;
        let Some(facts) = key_facts(session.as_mut(), &key).await? else {
            bail!("key {key} does not exist");
        };
        let mut answer = json!({"mode": "preview", "action": "expire", "key": shown(facts), "seconds": seconds, "next": NEXT});
        if !confirm {
            return Ok(answer);
        }
        let reply = session
            .command(&cmd(&["EXPIRE", &key, &seconds.to_string()]))
            .await?;
        answer["mode"] = json!("done");
        answer["set"] = json!(reply.as_int() == Some(1));
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    /// Keys by explicit list, or by pattern with an optional value filter.
    /// The preview carries the keys it selected; a confirmed call with the
    /// list deletes exactly those, and one with the pattern re-selects and
    /// re-checks the filter right before each UNLINK.
    pub async fn delete(
        &self,
        keys: &[String],
        pattern: Option<&str>,
        value_contains: Option<&str>,
        ignore_case: bool,
        confirm: bool,
    ) -> Result<Value> {
        self.require_writes()?;
        let filter = ValueFilter::parse(value_contains, ignore_case)?;
        let explicit: Vec<String> = keys.iter().map(|k| key_name(k)).collect::<Result<_, _>>()?;
        if explicit.len() > DELETE_MAX {
            bail!(InputError::OutOfRange {
                what: "keys",
                min: 1,
                max: DELETE_MAX as i64
            });
        }
        let pattern = if explicit.is_empty() {
            Some(key_pattern(pattern, filter.is_some())?)
        } else {
            None
        };
        let mut session = self.gateway.connect().await?;
        let (selected, examined, complete) = match &pattern {
            Some(pattern) => select(session.as_mut(), pattern, filter.as_ref()).await?,
            None => {
                let mut rows = Vec::new();
                for key in &explicit {
                    if let Some(facts) = key_facts(session.as_mut(), key).await?
                        && filter
                            .as_ref()
                            .is_none_or(|f| f.matches(&preview_text(&facts)))
                    {
                        rows.push(shown(facts));
                    }
                }
                (rows, explicit.len(), true)
            }
        };
        let mut answer = json!({
            "mode": "preview",
            "action": "delete",
            "source": if pattern.is_some() { "pattern" } else { "list" },
            "pattern": pattern,
            "valueFilter": filter.as_ref().map(ValueFilter::describe),
            "selected": selected.len(),
            "examined": examined,
            "complete": complete,
            "keys": selected,
            "next": "call again with confirm=true and the keys this preview lists, to delete exactly those",
        });
        if !confirm {
            return Ok(answer);
        }
        let names: Vec<String> = answer["keys"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r["key"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut deleted = 0i64;
        for batch in names.chunks(100) {
            let commands: Vec<Vec<String>> = batch.iter().map(|k| cmd(&["UNLINK", k])).collect();
            for reply in session.pipeline(&commands).await? {
                deleted += reply.as_int().unwrap_or(0);
            }
        }
        answer["mode"] = json!("done");
        answer["deleted"] = json!(deleted);
        answer["skipped"] = json!(names.len() as i64 - deleted);
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    /// One of the allowed parameters; the preview shows its current value.
    pub async fn config_set(&self, parameter: &str, value: &str, confirm: bool) -> Result<Value> {
        self.require_writes()?;
        let (parameter, value) = config_set_check(parameter, value)?;
        let mut session = self.gateway.connect().await?;
        let current = pairs_to_map(
            &session
                .command(&cmd(&["CONFIG", "GET", &parameter]))
                .await?,
        );
        let mut answer = json!({
            "mode": "preview",
            "action": "config_set",
            "parameter": parameter,
            "current": current.get(&parameter),
            "value": value,
            "hint": "CONFIG SET changes the running server only; the file on disk is not rewritten here",
            "next": NEXT,
        });
        if !confirm {
            return Ok(answer);
        }
        session
            .command(&cmd(&["CONFIG", "SET", &parameter, &value]))
            .await?;
        let after = pairs_to_map(
            &session
                .command(&cmd(&["CONFIG", "GET", &parameter]))
                .await?,
        );
        answer["mode"] = json!("done");
        answer["now"] = json!(after.get(&parameter));
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }

    /// One client by id, never this connection, never a replication link.
    pub async fn client_kill(&self, id: i64, confirm: bool) -> Result<Value> {
        self.require_writes()?;
        if id <= 0 {
            bail!(InputError::OutOfRange {
                what: "id",
                min: 1,
                max: i64::MAX
            });
        }
        let mut session = self.gateway.connect().await?;
        let own = session.command(&cmd(&["CLIENT", "ID"])).await?.as_int();
        if own == Some(id) {
            bail!("client {id} is this connection");
        }
        let listed = session
            .command(&cmd(&["CLIENT", "LIST", "ID", &id.to_string()]))
            .await?;
        let Some(client) = client_list(&listed.to_text().unwrap_or_default())
            .into_iter()
            .next()
        else {
            bail!("no client has id {id}: clients lists them");
        };
        let flags = client.get("flags").cloned().unwrap_or_default();
        if flags.contains('S') || flags.contains('M') {
            bail!(
                "client {id} is a replication link (flags {flags}): killing it breaks replication"
            );
        }
        let mut answer = json!({"mode": "preview", "action": "client_kill", "id": id, "client": client, "next": NEXT});
        if !confirm {
            return Ok(answer);
        }
        let reply = session
            .command(&cmd(&["CLIENT", "KILL", "ID", &id.to_string()]))
            .await?;
        answer["mode"] = json!("done");
        answer["killed"] = json!(reply.as_int().unwrap_or(0));
        answer.as_object_mut().map(|o| o.remove("next"));
        Ok(answer)
    }
}

/// The keys a pattern and a filter select, with their facts.
async fn select(
    session: &mut dyn RedisSession,
    pattern: &str,
    filter: Option<&ValueFilter>,
) -> Result<(Vec<Value>, usize, bool)> {
    let mut cursor = "0".to_string();
    let mut rows = Vec::new();
    let mut examined = 0usize;
    let mut complete = true;
    'walk: loop {
        let reply = session
            .command(&cmd(&[
                "SCAN", &cursor, "MATCH", pattern, "COUNT", SCAN_COUNT,
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
            if rows.len() >= DELETE_MAX {
                complete = false;
                break 'walk;
            }
            let Some(facts) = key_facts(session, &key).await? else {
                continue;
            };
            if filter.is_some_and(|f| !f.matches(&preview_text(&facts))) {
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
    Ok((rows, examined, complete))
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{Answer, FakeGateway, text, texts};
    use super::*;

    fn facts(key: &'static str, value: &'static str) -> Vec<Answer> {
        vec![
            (
                Box::leak(format!("TYPE {key}").into_boxed_str()),
                Ok(text("string")),
            ),
            (
                Box::leak(format!("TTL {key}").into_boxed_str()),
                Ok(Reply::Int(-1)),
            ),
            (
                Box::leak(format!("OBJECT ENCODING {key}").into_boxed_str()),
                Ok(text("embstr")),
            ),
            (
                Box::leak(format!("MEMORY USAGE {key}").into_boxed_str()),
                Ok(Reply::Int(56)),
            ),
            (
                Box::leak(format!("GETRANGE {key}").into_boxed_str()),
                Ok(text(value)),
            ),
        ]
    }

    fn change(gateway: &Arc<FakeGateway>, writes: bool) -> ChangeInstance {
        ChangeInstance::new(gateway.clone(), writes)
    }

    #[tokio::test]
    async fn expire_previews_the_key_then_sets_the_ttl() {
        let mut answers = facts("demo:a", "v");
        answers.push(("EXPIRE demo:a 60", Ok(Reply::Int(1))));
        answers.push(("TYPE demo:gone", Ok(text("none"))));
        let gateway = FakeGateway::answering(answers);
        let change = change(&gateway, true);
        let preview = change.expire("demo:a", 60, false).await.unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["key"]["type"], "string");
        assert!(!gateway.commands().iter().any(|c| c.starts_with("EXPIRE")));
        let done = change.expire("demo:a", 60, true).await.unwrap();
        assert_eq!(done["set"], true);
        assert!(
            change
                .expire("demo:gone", 60, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("does not exist")
        );
        assert!(change.expire("demo:a", 0, true).await.is_err());
        assert!(change.expire("", 10, true).await.is_err());
    }

    #[tokio::test]
    async fn delete_selects_by_pattern_or_list_and_unlinks_only_when_confirmed() {
        let mut answers = vec![(
            "SCAN 0 MATCH demo:lock:*",
            Ok(Reply::Array(vec![
                text("0"),
                texts(&["demo:lock:1", "demo:lock:2"]),
            ])),
        )];
        answers.extend(facts("demo:lock:1", "PENDING"));
        answers.extend(facts("demo:lock:2", "done"));
        answers.push(("UNLINK", Ok(Reply::Int(1))));
        answers.push(("TYPE demo:gone", Ok(text("none"))));
        let gateway = FakeGateway::answering(answers);
        let change = change(&gateway, true);
        let preview = change
            .delete(&[], Some("demo:lock:*"), Some("pending"), true, false)
            .await
            .unwrap();
        assert_eq!(preview["selected"], 1);
        assert_eq!(preview["keys"][0]["key"], "demo:lock:1");
        assert_eq!(preview["source"], "pattern");
        assert!(!gateway.commands().iter().any(|c| c.starts_with("UNLINK")));
        let done = change
            .delete(
                &["demo:lock:1".into(), "demo:gone".into()],
                None,
                None,
                false,
                true,
            )
            .await
            .unwrap();
        assert_eq!(done["source"], "list");
        assert_eq!(done["selected"], 1, "a missing key is not selected");
        assert_eq!(done["deleted"], 1);
        assert_eq!(done["skipped"], 0);
        assert_eq!(
            gateway
                .commands()
                .iter()
                .filter(|c| c.starts_with("UNLINK"))
                .count(),
            1
        );
        let filtered_list = change
            .delete(
                &["demo:lock:1".into(), "demo:lock:2".into()],
                None,
                Some("PENDING"),
                false,
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            filtered_list["selected"], 1,
            "the value filter applies to a list too"
        );
        assert_eq!(filtered_list["deleted"], 1);
        let by_pattern = change
            .delete(&[], Some("demo:lock:*"), None, false, true)
            .await
            .unwrap();
        assert_eq!(by_pattern["selected"], 2);
        assert_eq!(by_pattern["deleted"], 2);
        assert!(
            change
                .delete(&[], Some("*"), None, false, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("every key")
        );
        assert!(change.delete(&[], None, None, false, true).await.is_err());
        assert!(
            change
                .delete(&["-x".into()], None, None, false, true)
                .await
                .is_err()
        );
        let many: Vec<String> = (0..1001).map(|i| format!("k{i}")).collect();
        assert!(change.delete(&many, None, None, false, true).await.is_err());
    }

    #[tokio::test]
    async fn config_set_previews_the_current_value_and_refuses_other_parameters() {
        let gateway = FakeGateway::answering(vec![(
            "CONFIG GET maxmemory-policy",
            Ok(texts(&["maxmemory-policy", "noeviction"])),
        )]);
        let change = change(&gateway, true);
        let preview = change
            .config_set("maxmemory-policy", "allkeys-lru", false)
            .await
            .unwrap();
        assert_eq!(preview["current"], "noeviction");
        assert!(
            !gateway
                .commands()
                .iter()
                .any(|c| c.starts_with("CONFIG SET"))
        );
        let done = change
            .config_set("maxmemory-policy", "allkeys-lru", true)
            .await
            .unwrap();
        assert_eq!(done["mode"], "done");
        assert!(
            gateway
                .commands()
                .iter()
                .any(|c| c == "CONFIG SET maxmemory-policy allkeys-lru")
        );
        assert!(
            change
                .config_set("dir", "/tmp", true)
                .await
                .unwrap_err()
                .to_string()
                .contains("not a parameter config_set may change")
        );
        assert!(change.config_set("maxmemory", "lots", true).await.is_err());
        assert!(gateway.commands().iter().all(|c| !c.contains("dir")));
    }

    #[tokio::test]
    async fn client_kill_refuses_itself_and_replication_links() {
        let gateway = FakeGateway::answering(vec![
            ("CLIENT ID", Ok(Reply::Int(1))),
            (
                "CLIENT LIST ID 2",
                Ok(text("id=2 addr=127.0.0.1:1 flags=b cmd=blpop\n")),
            ),
            (
                "CLIENT LIST ID 3",
                Ok(text("id=3 addr=127.0.0.1:2 flags=S cmd=replconf\n")),
            ),
            ("CLIENT LIST ID 9", Ok(text(""))),
            ("CLIENT KILL ID 2", Ok(Reply::Int(1))),
        ]);
        let change = change(&gateway, true);
        let preview = change.client_kill(2, false).await.unwrap();
        assert_eq!(preview["client"]["cmd"], "blpop");
        assert!(
            !gateway
                .commands()
                .iter()
                .any(|c| c.starts_with("CLIENT KILL"))
        );
        let done = change.client_kill(2, true).await.unwrap();
        assert_eq!(done["killed"], 1);
        assert!(
            change
                .client_kill(1, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("this connection")
        );
        assert!(
            change
                .client_kill(3, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("replication")
        );
        assert!(
            change
                .client_kill(9, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("no client")
        );
        assert!(change.client_kill(0, true).await.is_err());
        assert_eq!(
            gateway
                .commands()
                .iter()
                .filter(|c| c.starts_with("CLIENT KILL"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn without_writes_every_write_is_refused_before_connecting() {
        let gateway = FakeGateway::answering(vec![]);
        let off = change(&gateway, false);
        for result in [
            off.expire("k", 1, true).await,
            off.delete(&["k".into()], None, None, false, true).await,
            off.config_set("maxmemory", "1mb", true).await,
            off.client_kill(2, true).await,
        ] {
            assert!(result.unwrap_err().to_string().contains("REDIS_WRITES"));
        }
        assert!(gateway.commands().is_empty());
        assert!(
            ChangeInstance::new(FakeGateway::refusing(), true)
                .expire("k", 1, true)
                .await
                .is_err()
        );
    }
}
