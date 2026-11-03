use std::collections::BTreeMap;

use serde_json::{Value, json};

const ALERTS_MAX: usize = 100;

/// Alertmanager's `/api/v2/alerts`, summarised: name, state, severity,
/// namespace, summary, since when; counts by state; a hundred at most.
#[must_use]
pub fn alerts(response: &Value, firing_only: bool) -> Value {
    let all: Vec<&Value> = response
        .as_array()
        .into_iter()
        .flatten()
        .filter(|alert| !firing_only || alert["status"]["state"] == "active")
        .collect();
    let mut by_state: BTreeMap<String, usize> = BTreeMap::new();
    for alert in &all {
        *by_state
            .entry(
                alert["status"]["state"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
            )
            .or_default() += 1;
    }
    let shown: Vec<Value> = all
        .iter()
        .take(ALERTS_MAX)
        .map(|alert| {
            let labels = &alert["labels"];
            json!({
                "name": labels["alertname"],
                "state": alert["status"]["state"],
                "severity": labels["severity"],
                "namespace": labels["namespace"],
                "summary": alert["annotations"]["summary"].as_str().or(alert["annotations"]["description"].as_str()),
                "startsAt": alert["startsAt"],
                "labels": labels,
            })
        })
        .collect();
    json!({
        "count": all.len(),
        "byState": by_state,
        "truncated": all.len() > shown.len(),
        "alerts": shown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> Value {
        json!([
            {"labels": {"alertname": "PodCrashLooping", "severity": "warning", "namespace": "shop"}, "annotations": {"summary": "web is crash looping"}, "status": {"state": "active"}, "startsAt": "2026-11-03T09:00:00Z"},
            {"labels": {"alertname": "Watchdog"}, "annotations": {"description": "always firing"}, "status": {"state": "suppressed"}},
            {"labels": {"alertname": "Old"}, "status": {}}
        ])
    }

    #[test]
    fn firing_only_keeps_the_active_ones() {
        let answer = alerts(&response(), true);
        assert_eq!(answer["count"], 1);
        assert_eq!(answer["alerts"][0]["name"], "PodCrashLooping");
        assert_eq!(answer["alerts"][0]["summary"], "web is crash looping");
        assert_eq!(answer["byState"]["active"], 1);
    }

    #[test]
    fn everything_is_counted_by_state_and_bounded() {
        let answer = alerts(&response(), false);
        assert_eq!(answer["count"], 3);
        assert_eq!(answer["byState"]["suppressed"], 1);
        assert_eq!(answer["byState"]["unknown"], 1);
        assert_eq!(answer["alerts"][1]["summary"], "always firing");
        let many: Vec<Value> = (0..120)
            .map(|_| json!({"status": {"state": "active"}}))
            .collect();
        let bounded = alerts(&json!(many), true);
        assert_eq!(bounded["truncated"], true);
        assert_eq!(bounded["alerts"].as_array().unwrap().len(), 100);
        assert_eq!(alerts(&json!({"message": "x"}), true)["count"], 0);
    }
}
