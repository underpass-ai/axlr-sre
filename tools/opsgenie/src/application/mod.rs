mod change_alerts;
mod diagnose_setup;
mod discover_scope;
mod inspect_alerts;
mod mirror_incidents;
mod read_alerts;

pub use change_alerts::ChangeAlerts;
pub use diagnose_setup::DiagnoseSetup;
pub use discover_scope::DiscoverScope;
pub use inspect_alerts::InspectAlerts;
pub use mirror_incidents::{MirrorIncidents, TickReport};
pub use read_alerts::ReadAlerts;

#[cfg(test)]
pub(crate) mod test_doubles {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use crate::domain::{IncidentDraft, MirrorState, Request};
    use crate::ports::{IncidentSink, OpsgenieClient, StateStore};

    pub(crate) const TEAM_ID: &str = "11111111-1111-4111-8111-111111111111";
    pub(crate) const ESC_ID: &str = "22222222-2222-4222-8222-222222222222";

    /// An open alert of the team, as the listing answers it.
    pub(crate) fn ours(tiny: u32, priority: &str) -> Value {
        json!({
            "id": format!("00000000-0000-4000-8000-{tiny:012}"),
            "tinyId": tiny.to_string(),
            "message": format!("Disk full on node-{tiny}"),
            "status": "open",
            "acknowledged": false,
            "priority": priority,
            "count": 1,
            "createdAt": "2026-11-03T08:15:42.064Z",
            "source": "prometheus",
            "tags": ["disk"],
            "responders": [{"type": "team", "id": TEAM_ID}],
        })
    }

    pub(crate) fn theirs(tiny: u32) -> Value {
        let mut alert = ours(tiny, "P3");
        alert["responders"] = json!([{"type": "team", "id": "other-team"}]);
        alert
    }

    pub(crate) fn listings() -> Vec<(&'static str, Result<Value, String>)> {
        vec![
            (
                "/v2/teams",
                Ok(json!({"data": [{"id": TEAM_ID, "name": "Platform"}]})),
            ),
            (
                "/v2/escalations",
                Ok(json!({"data": [{"id": ESC_ID, "name": "Critical"}]})),
            ),
        ]
    }

    /// Answers by exact path first, then by prefix; records every request.
    pub(crate) struct FakeClient {
        pub answers: Mutex<Vec<(&'static str, Result<Value, String>)>>,
        pub calls: Mutex<Vec<Request>>,
    }

    impl FakeClient {
        pub fn answering(answers: Vec<(&'static str, Result<Value, String>)>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                calls: Mutex::new(Vec::new()),
            })
        }

        pub fn set(&self, answers: Vec<(&'static str, Result<Value, String>)>) {
            *self.answers.lock().unwrap() = answers;
        }

        pub fn paths(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(Request::path)
                .collect()
        }

        pub fn writes(&self) -> Vec<Request> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.is_write())
                .cloned()
                .collect()
        }
    }

    #[async_trait]
    impl OpsgenieClient for FakeClient {
        async fn send(&self, request: &Request) -> Result<Value> {
            let path = request.path();
            self.calls.lock().unwrap().push(request.clone());
            let answers = self.answers.lock().unwrap();
            let found = answers
                .iter()
                .find(|(p, _)| *p == path)
                .or_else(|| answers.iter().find(|(p, _)| path.starts_with(p)));
            match found {
                Some((_, Ok(value))) => Ok(value.clone()),
                Some((_, Err(message))) => Err(anyhow!("{message}")),
                None => Err(anyhow!("unexpected request: {path}")),
            }
        }
    }

    /// A page of alerts for the fake: `/v2/alerts` with the given items.
    pub(crate) fn page(items: Vec<Value>) -> (&'static str, Result<Value, String>) {
        ("/v2/alerts", Ok(json!({"data": items})))
    }

    #[derive(Default)]
    pub(crate) struct FakeSink {
        pub opened: Mutex<Vec<IncidentDraft>>,
        pub resolved: Mutex<Vec<(String, String)>>,
        pub open_fails: bool,
        pub resolve_fails: bool,
    }

    #[async_trait]
    impl IncidentSink for FakeSink {
        async fn open(&self, draft: &IncidentDraft) -> Result<String> {
            if self.open_fails {
                return Err(anyhow!("bot paused"));
            }
            self.opened.lock().unwrap().push(draft.clone());
            Ok(format!("inc-{}", draft.fingerprint))
        }

        async fn resolve(&self, incident_id: &str, reason: &str) -> Result<()> {
            if self.resolve_fails {
                return Err(anyhow!("not found"));
            }
            self.resolved
                .lock()
                .unwrap()
                .push((incident_id.to_string(), reason.to_string()));
            Ok(())
        }
    }

    #[derive(Default)]
    pub(crate) struct FakeStore {
        pub state: Mutex<MirrorState>,
        pub saves: Mutex<usize>,
        pub load_fails: bool,
        pub save_fails: bool,
    }

    impl FakeStore {
        pub fn with(state: MirrorState) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(state),
                ..Self::default()
            })
        }

        pub fn state(&self) -> MirrorState {
            self.state.lock().unwrap().clone()
        }
    }

    impl StateStore for FakeStore {
        fn load(&self) -> Result<MirrorState> {
            if self.load_fails {
                return Err(anyhow!("state unreadable"));
            }
            Ok(self.state.lock().unwrap().clone())
        }

        fn save(&self, state: &MirrorState) -> Result<()> {
            if self.save_fails {
                return Err(anyhow!("disk full"));
            }
            *self.state.lock().unwrap() = state.clone();
            *self.saves.lock().unwrap() += 1;
            Ok(())
        }
    }
}
