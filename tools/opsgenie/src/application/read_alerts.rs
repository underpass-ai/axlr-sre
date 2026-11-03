use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::domain::{
    Alert, Directory, MAX_PAGES, Named, OpenAlerts, PAGE_SIZE, Request, ResolvedScope, Scope,
    ScopedAlert,
};
use crate::ports::OpsgenieClient;

/// The open alerts of the scope: every page of `status: open`, the
/// responders translated through the teams and escalations listings, and
/// the split between ours and the rest. Shared by the reads, the writes,
/// `discover` and the mirror.
pub struct ReadAlerts {
    client: Arc<dyn OpsgenieClient>,
    scope: Scope,
}

impl ReadAlerts {
    pub fn new(client: Arc<dyn OpsgenieClient>, scope: Scope) -> Self {
        Self { client, scope }
    }

    #[must_use]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The scope with ids. The listings are read only when a name needs
    /// them; a listing the key cannot read is an error only then.
    pub async fn resolve_scope(&self) -> Result<ResolvedScope> {
        let directory = self.directory().await?;
        Ok(directory.resolve(&self.scope)?)
    }

    /// Teams and escalations by id. Without names in the scope the listings
    /// are still tried, so `alert` can name the responders, but a refusal is
    /// not an error.
    pub async fn directory(&self) -> Result<Directory> {
        let teams = self.client.send(&Request::teams()).await;
        let escalations = self.client.send(&Request::escalations()).await;
        if self.scope.needs_names() {
            let teams = teams.context("the teams listing is needed to resolve the scope's names: use ids, or a key that can read teams")?;
            let escalations = escalations.context("the escalations listing is needed to resolve the scope's names: use ids, or a key that can read escalations")?;
            return Ok(Directory::from_listings(Some(&teams), Some(&escalations)));
        }
        Ok(Directory::from_listings(
            teams.as_ref().ok(),
            escalations.as_ref().ok(),
        ))
    }

    /// Every open alert, paged oldest first, deduplicated by id.
    pub async fn open(&self) -> Result<OpenAlerts> {
        let directory = self.directory().await?;
        let resolved = directory.resolve(&self.scope)?;
        let mut open = OpenAlerts {
            complete: true,
            ..OpenAlerts::default()
        };
        let mut seen = std::collections::HashSet::new();
        for page in 0..MAX_PAGES {
            let answer = self.client.send(&Request::open_alerts_page(page)).await?;
            let items = answer["data"].as_array().cloned().unwrap_or_default();
            let count = items.len();
            for item in &items {
                self.classify(item, &resolved, &mut open, &mut seen);
            }
            if count < PAGE_SIZE as usize {
                return Ok(open);
            }
        }
        open.complete = false;
        Ok(open)
    }

    fn classify(
        &self,
        item: &Value,
        resolved: &ResolvedScope,
        open: &mut OpenAlerts,
        seen: &mut std::collections::HashSet<String>,
    ) {
        let Some(alert) = Alert::read(item) else {
            open.unreadable += 1;
            return;
        };
        if !seen.insert(alert.id.clone()) {
            return;
        }
        match resolved.admits(&alert) {
            Some(via) => open.alerts.push(ScopedAlert { alert, via }),
            None => open.excluded += 1,
        }
    }

    /// The names of every team and escalation, for `alert`'s responders.
    pub async fn names(&self) -> Vec<(String, String)> {
        let Ok(directory) = self.directory().await else {
            return Vec::new();
        };
        let pairs = |items: Option<Vec<Named>>| {
            items
                .unwrap_or_default()
                .into_iter()
                .map(|n| (n.id, n.name))
                .collect::<Vec<_>>()
        };
        let mut names = pairs(directory.teams);
        names.extend(pairs(directory.escalations));
        names
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_doubles::{ESC_ID, FakeClient, TEAM_ID, listings, ours, page, theirs};
    use super::*;

    fn read(
        client: &Arc<FakeClient>,
        teams: Option<&str>,
        escalations: Option<&str>,
    ) -> ReadAlerts {
        let client: Arc<dyn OpsgenieClient> = client.clone();
        ReadAlerts::new(client, Scope::new(teams, escalations).unwrap())
    }

    #[tokio::test]
    async fn the_open_alerts_are_split_by_the_scope_and_unreadable_rows_counted() {
        let mut answers = listings();
        answers.push(page(vec![
            ours(1, "P1"),
            theirs(2),
            json!({"id": "broken"}),
            ours(1, "P1"),
        ]));
        let client = FakeClient::answering(answers);
        let open = read(&client, Some("platform"), None).open().await.unwrap();
        assert_eq!(open.alerts.len(), 1);
        assert_eq!(open.alerts[0].via, "Platform");
        assert_eq!((open.excluded, open.unreadable), (1, 1));
        assert!(open.complete);
        assert_eq!(
            client.paths()[..3],
            ["/v2/teams", "/v2/escalations", "/v2/alerts"]
        );
        assert_eq!(
            read(&client, Some("platform"), None).scope().teams(),
            ["platform"]
        );
        let names = read(&client, Some("platform"), None).names().await;
        assert_eq!(names.len(), 2);
        assert_eq!(names[1], (ESC_ID.to_string(), "Critical".to_string()));
    }

    #[tokio::test]
    async fn an_escalation_lets_an_alert_in_too() {
        let mut by_escalation = theirs(3);
        by_escalation["responders"] = json!([{"type": "escalation", "id": ESC_ID}]);
        let mut answers = listings();
        answers.push(page(vec![by_escalation]));
        let client = FakeClient::answering(answers);
        let open = read(&client, None, Some("critical")).open().await.unwrap();
        assert_eq!(open.alerts[0].via, "Critical");
    }

    #[tokio::test]
    async fn ids_need_no_listing_but_names_do() {
        let client = FakeClient::answering(vec![
            ("/v2/teams", Err("Opsgenie answered 403".into())),
            ("/v2/escalations", Err("Opsgenie answered 403".into())),
            page(vec![ours(1, "P2")]),
        ]);
        let open = read(&client, Some(TEAM_ID), None).open().await.unwrap();
        assert_eq!(open.alerts.len(), 1);
        assert_eq!(open.alerts[0].via, TEAM_ID);
        assert!(read(&client, Some(TEAM_ID), None).names().await.is_empty());
        let error = read(&client, Some("Platform"), None)
            .open()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("teams listing is needed"), "{error}");
        let scope = read(&client, Some(TEAM_ID), None)
            .resolve_scope()
            .await
            .unwrap();
        assert_eq!(scope.teams[0].id, TEAM_ID);
        let client = FakeClient::answering(listings());
        let error = read(&client, Some("Ghost"), None)
            .resolve_scope()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("`Ghost`"), "{error}");
        let client = FakeClient::answering(vec![
            ("/v2/teams", Ok(json!({"data": []}))),
            ("/v2/escalations", Err("Opsgenie answered 403".into())),
        ]);
        let error = read(&client, Some("x"), None)
            .resolve_scope()
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("escalations listing"), "{error}");
    }

    #[tokio::test]
    async fn twenty_full_pages_leave_the_listing_incomplete() {
        let full: Vec<Value> = (0..PAGE_SIZE).map(|n| ours(n, "P3")).collect();
        let mut answers = listings();
        answers.push(page(full));
        let client = FakeClient::answering(answers);
        let open = read(&client, Some("platform"), None).open().await.unwrap();
        assert!(!open.complete);
        assert_eq!(open.alerts.len(), PAGE_SIZE as usize);
        assert_eq!(
            client.paths().iter().filter(|p| *p == "/v2/alerts").count(),
            MAX_PAGES as usize
        );
        let client = FakeClient::answering(vec![
            (
                "/v2/teams",
                Ok(json!({"data": [{"id": TEAM_ID, "name": "Platform"}]})),
            ),
            ("/v2/escalations", Ok(json!({"data": []}))),
            ("/v2/alerts", Err("Opsgenie answered 500".into())),
        ]);
        assert!(read(&client, Some("platform"), None).open().await.is_err());
    }
}
