//! What every use case does with a session: build commands from checked
//! parts, read one key's facts, read INFO.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::domain::{Preview, Reply, info_sections, preview_command};
use crate::ports::RedisSession;

pub fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// `INFO <section>` as sections of key:value.
pub async fn info(
    session: &mut dyn RedisSession,
    section: &str,
) -> Result<std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>> {
    let reply = session.command(&cmd(&["INFO", section])).await?;
    let text = reply
        .to_text()
        .ok_or_else(|| anyhow!("INFO answered no text"))?;
    Ok(info_sections(&text))
}

/// One key's facts in one round trip: type, TTL, encoding, memory, and
/// the preview the type allows (a second round trip). `None` when the key
/// does not exist.
pub async fn key_facts(session: &mut dyn RedisSession, key: &str) -> Result<Option<Value>> {
    let replies = session
        .pipeline(&[
            cmd(&["TYPE", key]),
            cmd(&["TTL", key]),
            cmd(&["OBJECT", "ENCODING", key]),
            cmd(&["MEMORY", "USAGE", key, "SAMPLES", "0"]),
        ])
        .await?;
    let kind = replies
        .first()
        .and_then(Reply::to_text)
        .unwrap_or_else(|| "none".into());
    if kind == "none" {
        return Ok(None);
    }
    let ttl = replies.get(1).and_then(Reply::as_int).unwrap_or(-2);
    let mut facts = json!({
        "key": key,
        "type": kind,
        "ttlSeconds": if ttl >= 0 { json!(ttl) } else { Value::Null },
        "expires": ttl >= 0,
        "encoding": replies.get(2).and_then(Reply::to_text),
        "memoryBytes": replies.get(3).and_then(Reply::as_int),
    });
    if let Some(command) = preview_command(&kind, key) {
        let reply = session.command(&command).await?;
        let preview = Preview::of(&kind, &reply);
        facts["preview"] = preview.to_json();
        facts["previewText"] = json!(preview.text);
    }
    Ok(Some(facts))
}

/// The preview text of a key, for a value filter; empty when none.
pub fn preview_text(facts: &Value) -> String {
    facts["previewText"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The facts without the text used for filtering.
pub fn shown(mut facts: Value) -> Value {
    facts.as_object_mut().map(|o| o.remove("previewText"));
    facts
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeGateway, text, texts};
    use super::*;
    use crate::ports::RedisGateway;

    #[tokio::test]
    async fn a_key_is_read_in_one_pipeline_plus_its_preview() {
        let gateway = FakeGateway::answering(vec![
            ("TYPE demo:h", Ok(text("hash"))),
            ("TTL demo:h", Ok(Reply::Int(120))),
            ("OBJECT ENCODING demo:h", Ok(text("listpack"))),
            ("MEMORY USAGE demo:h", Ok(Reply::Int(96))),
            (
                "HSCAN demo:h",
                Ok(Reply::Array(vec![Reply::Int(0), texts(&["f", "v"])])),
            ),
            ("TYPE demo:gone", Ok(text("none"))),
            ("INFO memory", Ok(text("# Memory\r\nused_memory:10\r\n"))),
        ]);
        let mut session = gateway.connect().await.unwrap();
        let facts = key_facts(session.as_mut(), "demo:h")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(facts["type"], "hash");
        assert_eq!(facts["ttlSeconds"], 120);
        assert_eq!(facts["memoryBytes"], 96);
        assert_eq!(facts["preview"]["value"]["f"], "v");
        assert_eq!(preview_text(&facts), "f v");
        assert!(shown(facts).get("previewText").is_none());
        assert_eq!(
            key_facts(session.as_mut(), "demo:gone").await.unwrap(),
            None
        );
        assert_eq!(
            info(session.as_mut(), "memory").await.unwrap()["memory"]["used_memory"],
            "10"
        );
        assert_eq!(gateway.commands()[0], "TYPE demo:h");
        assert_eq!(gateway.commands()[4], "HSCAN demo:h 0 COUNT 50");
    }
}
