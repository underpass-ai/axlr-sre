use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::{MirrorConfigError, OpenAlerts, ScopedAlert};

const INTERVAL_MIN: u64 = 15;
const INTERVAL_MAX: u64 = 3_600;
const INTERVAL_DEFAULT: u64 = 60;
const ID_MAX: usize = 64;
const TITLE_CHARS: usize = 160;
const OBJECTIVE_MESSAGE_CHARS: usize = 1_000;
const STATE_VERSION: u64 = 1;

/// The mirror daemon's own variables: which tenant and bot the incidents go
/// to, where `axlr-bots` is, how often to look and where the memory lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorConfig {
    tenant: String,
    bot: String,
    axlr_bots: PathBuf,
    interval: Duration,
    state: PathBuf,
}

impl MirrorConfig {
    /// `OPSGENIE_MIRROR_TENANT`, `OPSGENIE_MIRROR_BOT`, `AXLR_BOTS` (the
    /// executable; `axlr-bots` on the PATH when unset), `OPSGENIE_MIRROR_INTERVAL`
    /// (seconds, 15 to 3600, 60 when unset), `OPSGENIE_MIRROR_STATE` (the state
    /// file; `$XDG_STATE_HOME/opsgenie-mirror/state.json` or
    /// `~/.local/state/opsgenie-mirror/state.json` when unset).
    pub fn from_env() -> Result<Self, MirrorConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, MirrorConfigError> {
        let id = |variable: &'static str| -> Result<String, MirrorConfigError> {
            let value = get(variable)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or(MirrorConfigError::Missing(variable))?;
            let valid = value.chars().count() <= ID_MAX
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
            if valid {
                Ok(value)
            } else {
                Err(MirrorConfigError::InvalidId(variable))
            }
        };
        let tenant = id("OPSGENIE_MIRROR_TENANT")?;
        let bot = id("OPSGENIE_MIRROR_BOT")?;
        let axlr_bots = get("AXLR_BOTS")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map_or_else(|| PathBuf::from("axlr-bots"), PathBuf::from);
        let interval = match get("OPSGENIE_MIRROR_INTERVAL")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            None => INTERVAL_DEFAULT,
            Some(raw) => match raw.parse::<u64>() {
                Ok(seconds) if (INTERVAL_MIN..=INTERVAL_MAX).contains(&seconds) => seconds,
                _ => {
                    return Err(MirrorConfigError::InvalidInterval {
                        min: INTERVAL_MIN,
                        max: INTERVAL_MAX,
                    });
                }
            },
        };
        let state = match get("OPSGENIE_MIRROR_STATE")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            Some(path) if path.chars().all(|c| c != '\0') => PathBuf::from(path),
            Some(_) => return Err(MirrorConfigError::InvalidState),
            None => {
                let base = get("XDG_STATE_HOME")
                    .filter(|v| !v.trim().is_empty())
                    .map(PathBuf::from)
                    .or_else(|| get("HOME").map(|home| PathBuf::from(home).join(".local/state")))
                    .ok_or(MirrorConfigError::InvalidState)?;
                base.join("opsgenie-mirror").join("state.json")
            }
        };
        Ok(Self {
            tenant,
            bot,
            axlr_bots,
            interval: Duration::from_secs(interval),
            state,
        })
    }

    #[must_use]
    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    #[must_use]
    pub fn bot(&self) -> &str {
        &self.bot
    }

    #[must_use]
    pub fn axlr_bots(&self) -> &PathBuf {
        &self.axlr_bots
    }

    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    #[must_use]
    pub fn state(&self) -> &PathBuf {
        &self.state
    }
}

/// The incident the mirror asks axlr-bots for when a new alert appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentDraft {
    pub tenant: String,
    pub bot: String,
    pub severity: &'static str,
    pub group_key: String,
    pub fingerprint: String,
    pub title: String,
    pub objective: String,
}

impl IncidentDraft {
    pub const SOURCE: &'static str = "opsgenie";

    #[must_use]
    pub fn for_alert(scoped: &ScopedAlert, tenant: &str, bot: &str) -> Self {
        let alert = &scoped.alert;
        Self {
            tenant: tenant.to_string(),
            bot: bot.to_string(),
            severity: alert.priority.severity(),
            group_key: format!("{}:{}", Self::SOURCE, alert.id),
            fingerprint: alert.tiny_id.clone(),
            title: format!(
                "{} (Opsgenie #{})",
                alert.one_line_message(TITLE_CHARS),
                alert.tiny_id
            ),
            objective: format!(
                "Opsgenie alert #{tiny} ({priority}), created {created}, in scope via {via}.\n\
                 Message: {message}\nSource: {source}\nOccurrences: {count}\nTags: {tags}",
                tiny = alert.tiny_id,
                priority = alert.priority,
                created = alert.created_label(),
                via = scoped.via,
                message = alert.one_line_message(OBJECTIVE_MESSAGE_CHARS),
                source = alert.source.as_deref().unwrap_or("?"),
                count = alert.count,
                tags = if alert.tags.is_empty() {
                    "-".to_string()
                } else {
                    alert.tags.join(", ")
                },
            ),
        }
    }
}

/// What one tick of the mirror has to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorPlan {
    /// The first tick: these alerts are noted as seen and nothing is opened.
    pub baseline: Option<Vec<String>>,
    pub open: Vec<ScopedAlert>,
    /// (alert id, incident id): the alert is no longer open.
    pub resolve: Vec<(String, String)>,
    /// Baseline alerts no longer open: forgotten, nothing to resolve.
    pub forget: Vec<String>,
    /// The listing was incomplete: nothing is resolved or forgotten from it.
    pub resolution_skipped: bool,
}

impl MirrorPlan {
    pub const RESOLVE_REASON: &'static str = "closed in Opsgenie";
}

/// The mirror's memory: which alerts it has seen and which incident it
/// opened for each; `None` is an alert of the baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorState {
    baseline_done: bool,
    seen: BTreeMap<String, Option<String>>,
}

impl MirrorState {
    #[must_use]
    pub fn restore(baseline_done: bool, seen: BTreeMap<String, Option<String>>) -> Self {
        Self {
            baseline_done,
            seen,
        }
    }

    #[must_use]
    pub fn baseline_done(&self) -> bool {
        self.baseline_done
    }

    #[must_use]
    pub fn seen(&self) -> &BTreeMap<String, Option<String>> {
        &self.seen
    }

    #[must_use]
    pub fn plan(&self, current: &OpenAlerts) -> MirrorPlan {
        let open_ids: Vec<&str> = current
            .alerts
            .iter()
            .map(|scoped| scoped.alert.id.as_str())
            .collect();
        if !self.baseline_done {
            return MirrorPlan {
                baseline: Some(open_ids.iter().map(|id| id.to_string()).collect()),
                ..MirrorPlan::default()
            };
        }
        let open: Vec<ScopedAlert> = current
            .alerts
            .iter()
            .filter(|scoped| !self.seen.contains_key(&scoped.alert.id))
            .cloned()
            .collect();
        if !current.complete {
            return MirrorPlan {
                open,
                resolution_skipped: true,
                ..MirrorPlan::default()
            };
        }
        let mut plan = MirrorPlan {
            open,
            ..MirrorPlan::default()
        };
        for (id, incident) in &self.seen {
            if open_ids.contains(&id.as_str()) {
                continue;
            }
            match incident {
                Some(incident) => plan.resolve.push((id.clone(), incident.clone())),
                None => plan.forget.push(id.clone()),
            }
        }
        plan
    }

    pub fn record_baseline(&mut self, ids: &[String]) {
        for id in ids {
            self.seen.insert(id.clone(), None);
        }
        self.baseline_done = true;
    }

    pub fn record_opened(&mut self, alert_id: &str, incident_id: &str) {
        self.seen
            .insert(alert_id.to_string(), Some(incident_id.to_string()));
    }

    pub fn forget(&mut self, alert_id: &str) {
        self.seen.remove(alert_id);
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"version": STATE_VERSION, "baseline_done": self.baseline_done, "seen": self.seen})
    }

    /// The file's JSON back; a shape that is not this one is an error.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        if value["version"].as_u64() != Some(STATE_VERSION) {
            return Err(format!("version is not {STATE_VERSION}"));
        }
        let baseline_done = value["baseline_done"]
            .as_bool()
            .ok_or("baseline_done is not a boolean")?;
        let mut seen = BTreeMap::new();
        for (alert, incident) in value["seen"].as_object().ok_or("seen is not an object")? {
            let incident = match incident {
                Value::Null => None,
                Value::String(id) if !id.trim().is_empty() => Some(id.clone()),
                _ => return Err(format!("seen.{alert} is not an incident id or null")),
            };
            seen.insert(alert.clone(), incident);
        }
        Ok(Self {
            baseline_done,
            seen,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Alert;

    fn scoped(tiny: u32, priority: &str) -> ScopedAlert {
        ScopedAlert {
            alert: Alert::read(&json!({
                "id": format!("id-{tiny}"),
                "tinyId": tiny.to_string(),
                "message": format!("Disk full on node-{tiny}"),
                "status": "open",
                "priority": priority,
                "count": 2,
                "createdAt": "2026-11-03T08:15:42.064Z",
                "source": "prometheus",
                "tags": ["disk", "node"],
            }))
            .unwrap(),
            via: "Platform".into(),
        }
    }

    fn open(numbers: &[u32], complete: bool) -> OpenAlerts {
        OpenAlerts {
            alerts: numbers.iter().map(|n| scoped(*n, "P2")).collect(),
            complete,
            ..OpenAlerts::default()
        }
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn the_configuration_names_each_missing_or_bad_variable() {
        let config = MirrorConfig::from_vars(vars(&[
            ("OPSGENIE_MIRROR_TENANT", " ops "),
            ("OPSGENIE_MIRROR_BOT", "triage"),
            ("HOME", "/home/x"),
        ]))
        .unwrap();
        assert_eq!((config.tenant(), config.bot()), ("ops", "triage"));
        assert_eq!(config.axlr_bots(), &PathBuf::from("axlr-bots"));
        assert_eq!(config.interval(), Duration::from_secs(60));
        assert_eq!(
            config.state(),
            &PathBuf::from("/home/x/.local/state/opsgenie-mirror/state.json")
        );
        let full = MirrorConfig::from_vars(vars(&[
            ("OPSGENIE_MIRROR_TENANT", "ops"),
            ("OPSGENIE_MIRROR_BOT", "triage"),
            ("AXLR_BOTS", "/opt/bin/axlr-bots"),
            ("OPSGENIE_MIRROR_INTERVAL", "120"),
            ("XDG_STATE_HOME", "/var/state"),
        ]))
        .unwrap();
        assert_eq!(full.axlr_bots(), &PathBuf::from("/opt/bin/axlr-bots"));
        assert_eq!(full.interval(), Duration::from_secs(120));
        assert_eq!(
            full.state(),
            &PathBuf::from("/var/state/opsgenie-mirror/state.json")
        );
        let explicit = MirrorConfig::from_vars(vars(&[
            ("OPSGENIE_MIRROR_TENANT", "ops"),
            ("OPSGENIE_MIRROR_BOT", "triage"),
            ("OPSGENIE_MIRROR_STATE", "/tmp/s.json"),
        ]))
        .unwrap();
        assert_eq!(explicit.state(), &PathBuf::from("/tmp/s.json"));

        for (pairs, expected) in [
            (
                vec![("OPSGENIE_MIRROR_BOT", "b")],
                MirrorConfigError::Missing("OPSGENIE_MIRROR_TENANT"),
            ),
            (
                vec![("OPSGENIE_MIRROR_TENANT", "t")],
                MirrorConfigError::Missing("OPSGENIE_MIRROR_BOT"),
            ),
            (
                vec![
                    ("OPSGENIE_MIRROR_TENANT", "t/x"),
                    ("OPSGENIE_MIRROR_BOT", "b"),
                ],
                MirrorConfigError::InvalidId("OPSGENIE_MIRROR_TENANT"),
            ),
            (
                vec![
                    ("OPSGENIE_MIRROR_TENANT", "t"),
                    ("OPSGENIE_MIRROR_BOT", "b"),
                    ("OPSGENIE_MIRROR_INTERVAL", "5"),
                    ("HOME", "/h"),
                ],
                MirrorConfigError::InvalidInterval { min: 15, max: 3600 },
            ),
            (
                vec![
                    ("OPSGENIE_MIRROR_TENANT", "t"),
                    ("OPSGENIE_MIRROR_BOT", "b"),
                    ("OPSGENIE_MIRROR_INTERVAL", "soon"),
                ],
                MirrorConfigError::InvalidInterval { min: 15, max: 3600 },
            ),
            (
                vec![
                    ("OPSGENIE_MIRROR_TENANT", "t"),
                    ("OPSGENIE_MIRROR_BOT", "b"),
                ],
                MirrorConfigError::InvalidState,
            ),
        ] {
            assert_eq!(MirrorConfig::from_vars(vars(&pairs)).unwrap_err(), expected);
        }
    }

    #[test]
    fn the_draft_carries_what_triage_needs() {
        let draft = IncidentDraft::for_alert(&scoped(7, "P4"), "ops", "triage");
        assert_eq!(
            (draft.tenant.as_str(), draft.bot.as_str()),
            ("ops", "triage")
        );
        assert_eq!(draft.severity, "sev4");
        assert_eq!(draft.group_key, "opsgenie:id-7");
        assert_eq!(draft.fingerprint, "7");
        assert_eq!(draft.title, "Disk full on node-7 (Opsgenie #7)");
        for expected in [
            "#7 (P4)",
            "2026-11-03T08:15Z",
            "via Platform",
            "Source: prometheus",
            "Occurrences: 2",
            "Tags: disk, node",
        ] {
            assert!(
                draft.objective.contains(expected),
                "{expected}: {}",
                draft.objective
            );
        }
        let mut bare = scoped(8, "P1");
        bare.alert.tags.clear();
        bare.alert.source = None;
        let objective = IncidentDraft::for_alert(&bare, "o", "b").objective;
        assert!(objective.contains("Source: ?") && objective.contains("Tags: -"));
    }

    #[test]
    fn the_first_tick_is_a_baseline_and_opens_nothing() {
        let mut state = MirrorState::default();
        let plan = state.plan(&open(&[1, 2], true));
        assert_eq!(plan.baseline.as_ref().unwrap().len(), 2);
        assert!(plan.open.is_empty());
        state.record_baseline(plan.baseline.as_ref().unwrap());
        assert!(state.baseline_done());
        assert_eq!(state.seen().len(), 2);
        assert!(state.plan(&open(&[1, 2], true)).open.is_empty());
    }

    #[test]
    fn a_new_alert_opens_and_a_gone_alert_resolves_or_is_forgotten() {
        let mut state = MirrorState::default();
        state.record_baseline(&["id-1".to_string()]);
        let plan = state.plan(&open(&[1, 3], true));
        assert_eq!(plan.open.len(), 1);
        assert_eq!(plan.open[0].alert.id, "id-3");
        state.record_opened("id-3", "inc-3");
        let plan = state.plan(&open(&[], true));
        assert_eq!(plan.resolve, [("id-3".to_string(), "inc-3".to_string())]);
        assert_eq!(plan.forget, ["id-1"]);
        assert!(!plan.resolution_skipped);
        state.forget("id-3");
        state.forget("id-1");
        assert!(state.seen().is_empty());
        assert_eq!(MirrorPlan::RESOLVE_REASON, "closed in Opsgenie");
    }

    #[test]
    fn an_incomplete_listing_resolves_nothing() {
        let mut state = MirrorState::default();
        state.record_baseline(&["id-1".to_string()]);
        state.record_opened("id-2", "inc-2");
        let plan = state.plan(&open(&[5], false));
        assert!(plan.resolution_skipped);
        assert!(plan.resolve.is_empty() && plan.forget.is_empty());
        assert_eq!(plan.open.len(), 1);
    }

    #[test]
    fn the_state_round_trips_through_json_and_refuses_other_shapes() {
        let mut state = MirrorState::default();
        state.record_baseline(&["id-1".to_string()]);
        state.record_opened("id-2", "inc-2");
        let value = state.to_json();
        assert_eq!(value["version"], 1);
        assert_eq!(value["seen"]["id-1"], Value::Null);
        assert_eq!(value["seen"]["id-2"], "inc-2");
        assert_eq!(MirrorState::from_json(&value).unwrap(), state);
        assert_eq!(MirrorState::restore(true, state.seen().clone()), state);
        for bad in [
            json!({}),
            json!({"version": 1}),
            json!({"version": 1, "baseline_done": true, "seen": []}),
            json!({"version": 1, "baseline_done": true, "seen": {"a": 3}}),
            json!({"version": 1, "baseline_done": true, "seen": {"a": " "}}),
        ] {
            assert!(MirrorState::from_json(&bad).is_err(), "{bad}");
        }
    }
}
