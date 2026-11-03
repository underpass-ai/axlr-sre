use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::ReadAlerts;
use crate::domain::{Alert, AlertRef, InputError, LIST_LIMIT_MAX, Request};
use crate::ports::OpsgenieClient;

const NOTE_CHARS: usize = 2_000;

/// The reads: `alerts_open`, `alert`, `alert_notes`. Only alerts of the
/// scope are shown; the rest are a count.
pub struct InspectAlerts {
    client: Arc<dyn OpsgenieClient>,
    read: Arc<ReadAlerts>,
}

impl InspectAlerts {
    pub fn new(client: Arc<dyn OpsgenieClient>, read: Arc<ReadAlerts>) -> Self {
        Self { client, read }
    }

    /// The open alerts of the scope, newest first, one page of them.
    pub async fn alerts_open(&self, limit: i64, offset: i64) -> Result<Value> {
        if !(1..=LIST_LIMIT_MAX).contains(&limit) {
            bail!(InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: LIST_LIMIT_MAX
            });
        }
        if offset < 0 {
            bail!(InputError::OutOfRange {
                what: "offset",
                min: 0,
                max: i64::MAX
            });
        }
        let open = self.read.open().await?;
        let mut alerts = open.alerts;
        alerts.sort_by(|a, b| b.alert.created_at.cmp(&a.alert.created_at));
        let total = alerts.len();
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let shown: Vec<Value> = alerts
            .iter()
            .skip(offset)
            .take(limit)
            .map(|scoped| scoped.alert.summary(&scoped.via))
            .collect();
        let next = (offset + shown.len() < total).then_some(offset + shown.len());
        Ok(json!({
            "total": total,
            "excluded": open.excluded,
            "count": shown.len(),
            "alerts": shown,
            "next_offset": next,
            "complete": open.complete,
            "unreadable": open.unreadable,
        }))
    }

    /// One alert of the scope with its description, details and responders.
    pub async fn alert(&self, reference: &str) -> Result<Value> {
        let reference = AlertRef::parse(reference)?;
        let scope = self.read.resolve_scope().await?;
        let answer = self.client.send(&Request::alert(&reference)).await?;
        let Some(alert) = Alert::read(&answer["data"]) else {
            bail!("Opsgenie answered without an alert for {reference}");
        };
        let Some(via) = scope.admits(&alert) else {
            bail!(InputError::OutOfScope(alert.tiny_id));
        };
        let names = self.read.names().await;
        Ok(alert.detail(&via, &names))
    }

    /// The notes of one alert of the scope, newest first.
    pub async fn alert_notes(&self, reference: &str, limit: i64) -> Result<Value> {
        let reference = AlertRef::parse(reference)?;
        let scope = self.read.resolve_scope().await?;
        let answer = self.client.send(&Request::alert(&reference)).await?;
        let Some(alert) = Alert::read(&answer["data"]) else {
            bail!("Opsgenie answered without an alert for {reference}");
        };
        if scope.admits(&alert).is_none() {
            bail!(InputError::OutOfScope(alert.tiny_id));
        }
        let notes = self
            .client
            .send(&Request::alert_notes(&alert.id, limit)?)
            .await?;
        let items: Vec<Value> = notes["data"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|n| {
                        let text = n["note"].as_str().unwrap_or_default();
                        let cut = text.chars().count() > NOTE_CHARS;
                        json!({
                            "note": text.chars().take(NOTE_CHARS).collect::<String>(),
                            "cut": cut,
                            "owner": n["owner"],
                            "created": n["createdAt"],
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({"tinyId": alert.tiny_id, "count": items.len(), "notes": items}))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeClient, listings, ours, page, theirs};
    use super::*;
    use crate::domain::Scope;

    fn inspect(client: &Arc<FakeClient>) -> InspectAlerts {
        let client: Arc<dyn OpsgenieClient> = client.clone();
        let read = Arc::new(ReadAlerts::new(
            client.clone(),
            Scope::new(Some("platform"), None).unwrap(),
        ));
        InspectAlerts::new(client, read)
    }

    #[tokio::test]
    async fn the_open_alerts_page_newest_first_within_the_scope() {
        let mut answers = listings();
        let mut older = ours(1, "P1");
        older["createdAt"] = json!("2026-11-01T00:00:00Z");
        answers.push(page(vec![older, ours(2, "P2"), ours(3, "P3"), theirs(4)]));
        let inspect = inspect(&FakeClient::answering(answers));
        let first = inspect.alerts_open(2, 0).await.unwrap();
        assert_eq!(first["total"], 3);
        assert_eq!(first["excluded"], 1);
        assert_eq!(first["count"], 2);
        assert_eq!(first["alerts"][0]["tinyId"], "2");
        assert_eq!(first["next_offset"], 2);
        let last = inspect.alerts_open(2, 2).await.unwrap();
        assert_eq!(last["alerts"][0]["tinyId"], "1");
        assert_eq!(last["next_offset"], Value::Null);
        for (limit, offset) in [(0, 0), (101, 0), (1, -1)] {
            assert!(inspect.alerts_open(limit, offset).await.is_err());
        }
    }

    #[tokio::test]
    async fn one_alert_is_detailed_and_a_foreign_one_refused() {
        let mut answers = listings();
        answers.push(("/v2/alerts/7", Ok(json!({"data": ours(7, "P2")}))));
        answers.push(("/v2/alerts/8", Ok(json!({"data": theirs(8)}))));
        answers.push(("/v2/alerts/9", Ok(json!({"data": {}}))));
        let client = FakeClient::answering(answers);
        let inspect = inspect(&client);
        let detail = inspect.alert("#7").await.unwrap();
        assert_eq!(detail["tinyId"], "7");
        assert_eq!(detail["responders"][0]["name"], "Platform");
        let error = inspect.alert("8").await.unwrap_err().to_string();
        assert!(error.contains("not in the scope"), "{error}");
        assert!(
            inspect
                .alert("9")
                .await
                .unwrap_err()
                .to_string()
                .contains("without an alert")
        );
        assert!(inspect.alert("x").await.is_err());
        assert!(client.writes().is_empty());
    }

    #[tokio::test]
    async fn the_notes_of_an_alert_are_bounded() {
        let mut answers = listings();
        answers.push(("/v2/alerts/7", Ok(json!({"data": ours(7, "P2")}))));
        answers.push(("/v2/alerts/8", Ok(json!({"data": theirs(8)}))));
        answers.push(("/v2/alerts/9", Ok(json!({"data": {}}))));
        answers.push((
            "/v2/alerts/00000000-0000-4000-8000-000000000007/notes",
            Ok(json!({"data": [{"note": "n".repeat(3000), "owner": "ana", "createdAt": "2026-11-03T09:00:00Z"}, {"note": "short"}]})),
        ));
        let inspect = inspect(&FakeClient::answering(answers));
        let notes = inspect.alert_notes("7", 10).await.unwrap();
        assert_eq!(notes["count"], 2);
        assert_eq!(notes["notes"][0]["cut"], true);
        assert_eq!(notes["notes"][0]["owner"], "ana");
        assert_eq!(notes["notes"][1]["note"], "short");
        assert!(inspect.alert_notes("8", 10).await.is_err());
        assert!(inspect.alert_notes("9", 10).await.is_err());
        assert!(inspect.alert_notes("7", 0).await.is_err());
        assert!(inspect.alert_notes("bad", 1).await.is_err());
    }
}
