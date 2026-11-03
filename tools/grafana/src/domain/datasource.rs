use serde_json::Value;

use super::InputError;

const UID_MAX: usize = 40;

/// A datasource uid as Grafana issues them, and as a call may name one.
pub fn validate_uid(value: &str) -> Result<String, InputError> {
    let value = value.trim();
    let shape = !value.is_empty()
        && value.len() <= UID_MAX
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if shape {
        Ok(value.to_string())
    } else {
        Err(InputError::InvalidUid(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasourceKind {
    Loki,
    Prometheus,
    Alertmanager,
    Other(String),
}

impl DatasourceKind {
    fn parse(value: &str) -> Self {
        match value {
            "loki" => Self::Loki,
            "prometheus" => Self::Prometheus,
            "alertmanager" => Self::Alertmanager,
            other => Self::Other(other.to_string()),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Loki => "loki",
            Self::Prometheus => "prometheus",
            Self::Alertmanager => "alertmanager",
            Self::Other(name) => name,
        }
    }

    /// The variable that fixes this kind's datasource, for the errors.
    #[must_use]
    pub fn variable(&self) -> &'static str {
        match self {
            Self::Loki => "GRAFANA_LOKI_DATASOURCE",
            Self::Prometheus => "GRAFANA_PROMETHEUS_DATASOURCE",
            _ => "the datasource argument",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Loki => "Loki",
            Self::Prometheus => "Prometheus",
            Self::Alertmanager => "Alertmanager",
            Self::Other(_) => "such",
        }
    }
}

/// A datasource as `GET /api/datasources` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datasource {
    pub uid: String,
    pub name: String,
    pub kind: DatasourceKind,
    pub is_default: bool,
}

impl Datasource {
    #[must_use]
    pub fn read_all(response: &Value) -> Vec<Self> {
        response
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(Self {
                            uid: item["uid"].as_str()?.to_string(),
                            name: item["name"].as_str().unwrap_or_default().to_string(),
                            kind: DatasourceKind::parse(item["type"].as_str().unwrap_or_default()),
                            is_default: item["isDefault"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The one datasource of a kind: the only one, or the default among several.
    pub fn choose<'a>(all: &'a [Self], kind: &DatasourceKind) -> Result<&'a Self, InputError> {
        let candidates: Vec<&Self> = all.iter().filter(|d| &d.kind == kind).collect();
        match candidates.as_slice() {
            [] => Err(InputError::NoDatasource {
                kind: kind.label(),
                variable: kind.variable(),
            }),
            [one] => Ok(one),
            many => many.iter().find(|d| d.is_default).copied().ok_or_else(|| {
                InputError::AmbiguousDatasource {
                    kind: kind.label(),
                    names: many
                        .iter()
                        .map(|d| format!("{} ({})", d.name, d.uid))
                        .collect::<Vec<_>>()
                        .join(", "),
                    variable: kind.variable(),
                }
            }),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({"uid": self.uid, "name": self.name, "type": self.kind.as_str(), "default": self.is_default})
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn listing() -> Vec<Datasource> {
        Datasource::read_all(&json!([
            {"uid": "loki-a", "name": "Loki", "type": "loki"},
            {"uid": "prom-a", "name": "Prom A", "type": "prometheus"},
            {"uid": "prom-b", "name": "Prom B", "type": "prometheus", "isDefault": true},
            {"uid": "am", "name": "AM", "type": "alertmanager"},
            {"uid": "tempo", "name": "Tempo", "type": "tempo"},
            {"name": "no uid", "type": "loki"}
        ]))
    }

    #[test]
    fn the_listing_is_read_and_the_kinds_named() {
        let all = listing();
        assert_eq!(all.len(), 5);
        assert_eq!(all[4].kind, DatasourceKind::Other("tempo".into()));
        assert_eq!(all[4].kind.as_str(), "tempo");
        assert_eq!(all[3].kind.as_str(), "alertmanager");
        assert_eq!(all[0].to_json()["type"], "loki");
        assert!(Datasource::read_all(&json!({"message": "forbidden"})).is_empty());
    }

    #[test]
    fn the_only_one_or_the_default_is_chosen() {
        let all = listing();
        assert_eq!(
            Datasource::choose(&all, &DatasourceKind::Loki).unwrap().uid,
            "loki-a"
        );
        assert_eq!(
            Datasource::choose(&all, &DatasourceKind::Prometheus)
                .unwrap()
                .uid,
            "prom-b"
        );
        let no_default: Vec<Datasource> = all
            .iter()
            .cloned()
            .map(|mut d| {
                d.is_default = false;
                d
            })
            .collect();
        assert!(
            matches!(Datasource::choose(&no_default, &DatasourceKind::Prometheus), Err(InputError::AmbiguousDatasource { names, .. }) if names.contains("Prom A (prom-a)"))
        );
        assert_eq!(
            Datasource::choose(&all, &DatasourceKind::Other("tempo".into()))
                .unwrap()
                .uid,
            "tempo"
        );
        assert!(matches!(
            Datasource::choose(&[], &DatasourceKind::Loki),
            Err(InputError::NoDatasource {
                variable: "GRAFANA_LOKI_DATASOURCE",
                ..
            })
        ));
        assert!(matches!(
            Datasource::choose(&[], &DatasourceKind::Alertmanager),
            Err(InputError::NoDatasource {
                variable: "the datasource argument",
                ..
            })
        ));
    }

    #[test]
    fn uids_keep_to_their_charset() {
        assert_eq!(
            validate_uid(" P1809F7CD0C75ACF3 ").unwrap(),
            "P1809F7CD0C75ACF3"
        );
        assert_eq!(validate_uid("loki_1-a").unwrap(), "loki_1-a");
        for bad in ["", "a b", "a/b", "../x", &"a".repeat(41)] {
            assert!(validate_uid(bad).is_err(), "{bad}");
        }
    }
}
