use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::ReadAlerts;
use crate::domain::{Alert, AlertRef, InputError, Note, Request};
use crate::ports::OpsgenieClient;

const NEXT: &str = "call again with confirm=true to run it";

/// The writes: `alert_acknowledge`, `alert_note`, `alert_close`. Every one
/// previews unless `confirm` is true; the preview path never sends a
/// write. Only an open alert of the scope can be changed. Opsgenie takes
/// writes asynchronously: the answer carries the request id and whether
/// Opsgenie had processed it when asked once.
pub struct ChangeAlerts {
    client: Arc<dyn OpsgenieClient>,
    read: Arc<ReadAlerts>,
    writes: bool,
    user: Option<String>,
}

impl ChangeAlerts {
    pub fn new(
        client: Arc<dyn OpsgenieClient>,
        read: Arc<ReadAlerts>,
        writes: bool,
        user: Option<String>,
    ) -> Self {
        Self {
            client,
            read,
            writes,
            user,
        }
    }

    pub async fn acknowledge(
        &self,
        reference: &str,
        note: Option<&str>,
        confirm: bool,
    ) -> Result<Value> {
        let note = match note.map(str::trim).filter(|n| !n.is_empty()) {
            Some(text) => Some(Note::parse(Some(text))?),
            None => None,
        };
        let (alert, mut answer) = self.target(reference, "acknowledge").await?;
        if alert["acknowledged"] == true {
            answer["already"] = json!(true);
        }
        if !confirm {
            return Ok(answer);
        }
        let request = Request::acknowledge(
            alert["id"].as_str().unwrap_or_default(),
            note.as_ref(),
            self.user.as_deref(),
        );
        self.send(request, answer).await
    }

    pub async fn note(&self, reference: &str, note: Option<&str>, confirm: bool) -> Result<Value> {
        let note = Note::parse(note)?;
        let (alert, mut answer) = self.target(reference, "note").await?;
        answer["note"] = json!(note.as_str());
        if !confirm {
            return Ok(answer);
        }
        let request = Request::add_note(
            alert["id"].as_str().unwrap_or_default(),
            &note,
            self.user.as_deref(),
        );
        self.send(request, answer).await
    }

    /// Closes with the note in the same call, so a close never lands without
    /// the note that says why.
    pub async fn close(&self, reference: &str, note: Option<&str>, confirm: bool) -> Result<Value> {
        let note = Note::parse(note)?;
        let (alert, mut answer) = self.target(reference, "close").await?;
        answer["note"] = json!(note.as_str());
        if !confirm {
            return Ok(answer);
        }
        let request = Request::close(
            alert["id"].as_str().unwrap_or_default(),
            &note,
            self.user.as_deref(),
        );
        self.send(request, answer).await
    }

    /// The alert, checked: writes on, in the scope, open. One GET of the
    /// alert and the scope's ids, never the whole listing, so an account
    /// with more open alerts than the listing walks still answers right.
    /// Returns its summary and the preview answer.
    async fn target(&self, reference: &str, action: &str) -> Result<(Value, Value)> {
        if !self.writes {
            bail!(InputError::WritesDisabled);
        }
        let reference = AlertRef::parse(reference)?;
        let scope = self.read.resolve_scope().await?;
        let answer = self.client.send(&Request::alert(&reference)).await?;
        let Some(alert) = Alert::read(&answer["data"]) else {
            bail!("Opsgenie answered without an alert for {reference}");
        };
        let Some(via) = scope.admits(&alert) else {
            bail!(InputError::OutOfScope(alert.tiny_id));
        };
        if !alert.is_open() {
            bail!(InputError::NotOpen {
                alert: alert.tiny_id,
                status: alert.status
            });
        }
        let summary = alert.summary(&via);
        let answer = json!({
            "mode": "preview",
            "action": action,
            "alert": summary,
            "user": self.user,
            "next": NEXT,
        });
        Ok((summary, answer))
    }

    async fn send(&self, request: Request, mut answer: Value) -> Result<Value> {
        let accepted = self.client.send(&request).await?;
        let request_id = accepted["requestId"].as_str().unwrap_or_default();
        answer["mode"] = json!("done");
        answer["requestId"] = json!(request_id);
        answer.as_object_mut().map(|o| o.remove("next"));
        if request_id.is_empty() {
            answer["processed"] = json!("unknown");
            return Ok(answer);
        }
        match self.client.send(&Request::request_status(request_id)).await {
            Ok(status) if status["data"]["isSuccess"] == true => {
                answer["processed"] = json!("verified");
            }
            Ok(status) if status["data"]["success"] == false => {
                answer["processed"] = json!("failed");
                answer["detail"] = status["data"]["status"].clone();
            }
            Ok(_) => {
                answer["processed"] = json!("pending");
                answer["hint"] = json!(
                    "Opsgenie processes writes asynchronously: read the alert again in a moment"
                );
            }
            Err(error) => {
                answer["processed"] = json!("unknown");
                answer["detail"] = json!(format!("{error:#}"));
            }
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeClient, listings, ours, theirs};
    use super::*;
    use crate::domain::Scope;

    fn change(client: &Arc<FakeClient>, writes: bool) -> ChangeAlerts {
        let client: Arc<dyn OpsgenieClient> = client.clone();
        let read = Arc::new(ReadAlerts::new(
            client.clone(),
            Scope::new(Some("platform"), None).unwrap(),
        ));
        ChangeAlerts::new(client, read, writes, Some("sre-bot".into()))
    }

    fn answers() -> Vec<(&'static str, Result<Value, String>)> {
        let mut answers = listings();
        let mut acked = ours(2, "P2");
        acked["acknowledged"] = json!(true);
        answers.push(("/v2/alerts/1", Ok(json!({"data": ours(1, "P1")}))));
        answers.push(("/v2/alerts/2", Ok(json!({"data": acked}))));
        answers.push(("/v2/alerts/8", Ok(json!({"data": theirs(8)}))));
        let mut closed = ours(9, "P3");
        closed["status"] = json!("closed");
        answers.push(("/v2/alerts/9", Ok(json!({"data": closed}))));
        answers.push(("/v2/alerts/66", Ok(json!({"data": {}}))));
        answers.push((
            "/v2/alerts/77",
            Err("Opsgenie answered 404 for /v2/alerts/77".into()),
        ));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/close",
            Ok(json!({"requestId": "r-1"})),
        ));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/notes",
            Ok(json!({"requestId": "r-2"})),
        ));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000002/acknowledge",
            Ok(json!({})),
        ));
        answers.push((
            "/v2/alerts/requests/r-1",
            Ok(json!({"data": {"isSuccess": true}})),
        ));
        answers.push((
            "/v2/alerts/requests/r-2",
            Ok(json!({"data": {"success": false, "status": "Alert does not exist"}})),
        ));
        answers
    }

    #[tokio::test]
    async fn a_preview_sends_no_write_and_the_confirmed_close_is_verified() {
        let client = FakeClient::answering(answers());
        let change = change(&client, true);
        let preview = change.close("#1", Some(" fixed "), false).await.unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["action"], "close");
        assert_eq!(preview["alert"]["tinyId"], "1");
        assert_eq!(preview["note"], "fixed");
        assert_eq!(preview["user"], "sre-bot");
        assert!(preview["next"].as_str().unwrap().contains("confirm=true"));
        assert!(client.writes().is_empty());

        let done = change.close("1", Some("fixed"), true).await.unwrap();
        assert_eq!(done["mode"], "done");
        assert_eq!(done["requestId"], "r-1");
        assert_eq!(done["processed"], "verified");
        assert_eq!(done["next"], Value::Null);
        let writes = client.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].body().unwrap()["note"], "fixed");
        assert_eq!(writes[0].body().unwrap()["user"], "sre-bot");
    }

    #[tokio::test]
    async fn a_note_whose_request_failed_says_so_and_an_acknowledge_without_id_is_unknown() {
        let client = FakeClient::answering(answers());
        let change = change(&client, true);
        let note = change.note("1", Some("looking"), true).await.unwrap();
        assert_eq!(note["processed"], "failed");
        assert_eq!(note["detail"], "Alert does not exist");
        let ack = change.acknowledge("2", None, true).await.unwrap();
        assert_eq!(ack["already"], true);
        assert_eq!(ack["processed"], "unknown");
        assert_eq!(client.writes().len(), 2);
        let preview = change.acknowledge("1", Some("on it"), false).await.unwrap();
        assert_eq!(preview["already"], Value::Null);
        assert_eq!(client.writes().len(), 2);
    }

    #[tokio::test]
    async fn pending_and_unreadable_request_status_are_reported() {
        let mut answers = answers();
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000002/notes",
            Ok(json!({"requestId": "r-3"})),
        ));
        answers.push(("/v2/alerts/requests/r-3", Ok(json!({"data": {}}))));
        let client = FakeClient::answering(answers);
        let pending = change(&client, true)
            .note("2", Some("x"), true)
            .await
            .unwrap();
        assert_eq!(pending["processed"], "pending");
        client.set({
            let mut a = answers_with_close_status(Err("Opsgenie answered 500".into()));
            a.push(("/v2/alerts/1", Ok(json!({"data": ours(1, "P1")}))));
            a
        });
        let unknown = change(&client, true)
            .close("1", Some("x"), true)
            .await
            .unwrap();
        assert_eq!(unknown["processed"], "unknown");
        assert!(unknown["detail"].as_str().unwrap().contains("500"));
    }

    fn answers_with_close_status(
        status: Result<Value, String>,
    ) -> Vec<(&'static str, Result<Value, String>)> {
        let mut answers = listings();
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000001/close",
            Ok(json!({"requestId": "r-1"})),
        ));
        answers.push(("/v2/alerts/requests/r-1", status));
        answers
    }

    #[tokio::test]
    async fn every_refusal_comes_before_any_write() {
        let client = FakeClient::answering(answers());
        let off = change(&client, false);
        assert!(
            off.close("1", Some("x"), true)
                .await
                .unwrap_err()
                .to_string()
                .contains("OPSGENIE_WRITES")
        );
        let change = change(&client, true);
        for (reference, note, expected) in [
            ("1", None, "note is required"),
            ("bad", Some("x"), "invalid alert"),
            ("8", Some("x"), "not in the scope"),
            ("9", Some("x"), "is closed, not open"),
            ("77", Some("x"), "404"),
            ("66", Some("x"), "without an alert"),
        ] {
            let error = change
                .close(reference, note, true)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{reference}: {error}");
        }
        assert!(change.note("1", None, false).await.is_err());
        assert!(
            change
                .acknowledge("1", Some(&"n".repeat(30_000)), false)
                .await
                .is_err()
        );
        assert!(client.writes().is_empty());
    }
}
