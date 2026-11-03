use serde_json::{Value, json};

use super::{Alert, ConfigError, InputError};

const ENTRY_MAX: usize = 100;

/// Which alerts are ours: those with one of these teams or one of these
/// escalations among their responders, each given by name or by id. An empty
/// scope is refused: it would be every alert of the account. Everything
/// outside stays out, for the writes too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    teams: Vec<String>,
    escalations: Vec<String>,
}

impl Scope {
    pub fn new(teams: Option<&str>, escalations: Option<&str>) -> Result<Self, ConfigError> {
        let teams = entries(teams, "OPSGENIE_TEAMS")?;
        let escalations = entries(escalations, "OPSGENIE_ESCALATIONS")?;
        if teams.is_empty() && escalations.is_empty() {
            return Err(ConfigError::EmptyScope);
        }
        Ok(Self { teams, escalations })
    }

    #[must_use]
    pub fn teams(&self) -> &[String] {
        &self.teams
    }

    #[must_use]
    pub fn escalations(&self) -> &[String] {
        &self.escalations
    }

    /// Whether any entry is a name, so the teams or escalations listing is
    /// needed to turn it into the id the alerts carry.
    #[must_use]
    pub fn needs_names(&self) -> bool {
        self.teams
            .iter()
            .chain(&self.escalations)
            .any(|entry| !looks_like_id(entry))
    }

    /// The scope as given, for `discover` and `doctor`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"teams": self.teams, "escalations": self.escalations})
    }
}

fn entries(raw: Option<&str>, variable: &'static str) -> Result<Vec<String>, ConfigError> {
    let mut entries = Vec::new();
    for entry in raw.unwrap_or_default().split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.chars().count() > ENTRY_MAX {
            return Err(ConfigError::InvalidScope(variable));
        }
        if !entries
            .iter()
            .any(|known: &String| known.eq_ignore_ascii_case(entry))
        {
            entries.push(entry.to_string());
        }
    }
    Ok(entries)
}

/// Opsgenie's ids are UUIDs.
#[must_use]
pub fn looks_like_id(entry: &str) -> bool {
    let parts: Vec<&str> = entry.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// One team or escalation with its id and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub id: String,
    pub name: String,
}

/// What `/v2/teams` and `/v2/escalations` list: the names behind the ids
/// the alerts carry. Either listing may be missing when the key cannot
/// read it; then only ids resolve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directory {
    pub teams: Option<Vec<Named>>,
    pub escalations: Option<Vec<Named>>,
}

impl Directory {
    pub fn from_listings(teams: Option<&Value>, escalations: Option<&Value>) -> Self {
        Self {
            teams: teams.map(read_named),
            escalations: escalations.map(read_named),
        }
    }

    /// Every entry of the scope as an id and a name. A name the listing does
    /// not have, or a name without a listing, is an error: the scope would
    /// silently shrink otherwise.
    pub fn resolve(&self, scope: &Scope) -> Result<ResolvedScope, InputError> {
        let mut unknown = Vec::new();
        let teams = resolve_entries(scope.teams(), self.teams.as_deref(), &mut unknown);
        let escalations = resolve_entries(
            scope.escalations(),
            self.escalations.as_deref(),
            &mut unknown,
        );
        if !unknown.is_empty() {
            return Err(InputError::UnknownScope(unknown.join(", ")));
        }
        Ok(ResolvedScope { teams, escalations })
    }
}

fn read_named(listing: &Value) -> Vec<Named> {
    listing["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Named {
                        id: item["id"].as_str()?.to_string(),
                        name: item["name"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_entries(
    entries: &[String],
    listing: Option<&[Named]>,
    unknown: &mut Vec<String>,
) -> Vec<Named> {
    let mut resolved = Vec::new();
    for entry in entries {
        let found = listing.and_then(|items| {
            items
                .iter()
                .find(|item| item.id == *entry || item.name.eq_ignore_ascii_case(entry))
        });
        match found {
            Some(named) => resolved.push(named.clone()),
            None if looks_like_id(entry) => resolved.push(Named {
                id: entry.clone(),
                name: entry.clone(),
            }),
            None => unknown.push(format!("`{entry}`")),
        }
    }
    resolved
}

/// The scope with every entry as an id: what classifies the alerts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedScope {
    pub teams: Vec<Named>,
    pub escalations: Vec<Named>,
}

impl ResolvedScope {
    /// The team or escalation name that lets the alert in, if any.
    #[must_use]
    pub fn admits(&self, alert: &Alert) -> Option<String> {
        alert.responders.iter().find_map(|responder| {
            let listing = match responder.kind.as_str() {
                "team" => &self.teams,
                "escalation" => &self.escalations,
                _ => return None,
            };
            listing
                .iter()
                .find(|named| named.id == responder.id)
                .map(|named| named.name.clone())
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let named = |items: &[Named]| -> Vec<Value> {
            items
                .iter()
                .map(|n| json!({"id": n.id, "name": n.name}))
                .collect()
        };
        json!({"teams": named(&self.teams), "escalations": named(&self.escalations)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Responder;

    const TEAM_ID: &str = "4513b7ea-3b91-438f-b7e4-e3e54af9147c";
    const ESC_ID: &str = "a1b2c3d4-0000-4000-8000-000000000001";

    fn alert(responders: Vec<Responder>) -> Alert {
        Alert::read(&json!({"id": "x", "tinyId": "1", "message": "m", "status": "open"}))
            .map(|mut alert| {
                alert.responders = responders;
                alert
            })
            .unwrap()
    }

    #[test]
    fn entries_are_split_trimmed_and_deduplicated_and_an_empty_scope_is_refused() {
        let scope = Scope::new(Some(" Platform ,, platform, Data"), Some(TEAM_ID)).unwrap();
        assert_eq!(scope.teams(), ["Platform", "Data"]);
        assert_eq!(scope.escalations(), [TEAM_ID]);
        assert!(scope.needs_names());
        assert!(!Scope::new(Some(TEAM_ID), None).unwrap().needs_names());
        assert_eq!(scope.to_json()["teams"][1], "Data");
        assert_eq!(
            Scope::new(Some(" , "), None).unwrap_err(),
            ConfigError::EmptyScope
        );
        assert_eq!(
            Scope::new(None, Some(&"e".repeat(101))).unwrap_err(),
            ConfigError::InvalidScope("OPSGENIE_ESCALATIONS")
        );
        assert!(looks_like_id(TEAM_ID));
        assert!(!looks_like_id("platform-team"));
        assert!(!looks_like_id("4513b7ea-3b91-438f-b7e4-e3e54af9147g"));
    }

    #[test]
    fn names_resolve_through_the_listings_and_ids_without_them() {
        let directory = Directory::from_listings(
            Some(
                &json!({"data": [{"id": TEAM_ID, "name": "Platform"}, {"id": "t2", "name": "Data"}]}),
            ),
            None,
        );
        let scope = Scope::new(Some("platform"), Some(ESC_ID)).unwrap();
        let resolved = directory.resolve(&scope).unwrap();
        assert_eq!(resolved.teams[0].id, TEAM_ID);
        assert_eq!(resolved.teams[0].name, "Platform");
        assert_eq!(resolved.escalations[0].name, ESC_ID);
        assert_eq!(resolved.to_json()["teams"][0]["name"], "Platform");

        let unknown = directory
            .resolve(&Scope::new(Some("Ghost"), Some("Critical")).unwrap())
            .unwrap_err();
        assert_eq!(
            unknown,
            InputError::UnknownScope("`Ghost`, `Critical`".into())
        );
        assert!(
            Directory::default()
                .resolve(&Scope::new(Some(TEAM_ID), None).unwrap())
                .is_ok()
        );
        assert_eq!(
            Directory::from_listings(Some(&json!({})), None).teams,
            Some(vec![])
        );
    }

    #[test]
    fn an_alert_is_in_when_a_team_or_escalation_responder_matches() {
        let resolved = ResolvedScope {
            teams: vec![Named {
                id: TEAM_ID.into(),
                name: "Platform".into(),
            }],
            escalations: vec![Named {
                id: ESC_ID.into(),
                name: "Critical".into(),
            }],
        };
        let by_team = alert(vec![Responder {
            kind: "team".into(),
            id: TEAM_ID.into(),
        }]);
        assert_eq!(resolved.admits(&by_team).as_deref(), Some("Platform"));
        let by_escalation = alert(vec![
            Responder {
                kind: "user".into(),
                id: TEAM_ID.into(),
            },
            Responder {
                kind: "escalation".into(),
                id: ESC_ID.into(),
            },
        ]);
        assert_eq!(resolved.admits(&by_escalation).as_deref(), Some("Critical"));
        let other = alert(vec![Responder {
            kind: "team".into(),
            id: "other".into(),
        }]);
        assert_eq!(resolved.admits(&other), None);
    }
}
