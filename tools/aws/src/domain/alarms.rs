use serde_json::{Value, json};

const REASON_CHARS: usize = 200;

/// The alarms of a `describe-alarms` answer, metric and composite ones,
/// each with its state, since when, why (bounded) and what it watches.
#[must_use]
pub fn alarms(answer: &Value) -> Vec<Value> {
    let metric = answer["MetricAlarms"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let composite = answer["CompositeAlarms"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    metric
        .iter()
        .map(|a| alarm(a, "metric"))
        .chain(composite.iter().map(|a| alarm(a, "composite")))
        .collect()
}

fn alarm(raw: &Value, kind: &str) -> Value {
    let reason = raw["StateReason"].as_str().unwrap_or_default();
    let cut = reason.chars().count() > REASON_CHARS;
    let watches = match kind {
        "metric" => {
            let dimensions: Vec<String> = raw["Dimensions"]
                .as_array()
                .map(|d| {
                    d.iter()
                        .filter_map(|x| {
                            Some(format!("{}={}", x["Name"].as_str()?, x["Value"].as_str()?))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut text = format!(
                "{} {} {}",
                raw["Namespace"].as_str().unwrap_or("?"),
                raw["MetricName"].as_str().unwrap_or("?"),
                raw["Statistic"]
                    .as_str()
                    .or(raw["ExtendedStatistic"].as_str())
                    .unwrap_or("")
            );
            if !dimensions.is_empty() {
                text.push_str(&format!(" {{{}}}", dimensions.join(", ")));
            }
            if let (Some(op), Some(threshold)) = (
                raw["ComparisonOperator"].as_str(),
                raw["Threshold"].as_f64(),
            ) {
                text.push_str(&format!(" {op} {threshold}"));
            }
            text.trim().to_string()
        }
        _ => raw["AlarmRule"].as_str().unwrap_or("?").to_string(),
    };
    json!({
        "name": raw["AlarmName"],
        "kind": kind,
        "state": raw["StateValue"],
        "since": raw["StateUpdatedTimestamp"],
        "reason": reason.chars().take(REASON_CHARS).collect::<String>(),
        "reasonCut": cut,
        "watches": watches,
        "actionsEnabled": raw["ActionsEnabled"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_and_composite_alarms_are_summarised() {
        let answer = json!({
            "MetricAlarms": [{
                "AlarmName": "demo-errors", "StateValue": "ALARM", "StateReason": "r".repeat(300),
                "StateUpdatedTimestamp": "2026-11-03T10:00:00+00:00", "Namespace": "AWS/Lambda",
                "MetricName": "Errors", "Statistic": "Sum", "Dimensions": [{"Name": "FunctionName", "Value": "demo-fn"}],
                "ComparisonOperator": "GreaterThanThreshold", "Threshold": 1.0, "ActionsEnabled": true
            }, {
                "AlarmName": "p99", "StateValue": "OK", "ExtendedStatistic": "p99", "Namespace": "AWS/Lambda", "MetricName": "Duration"
            }],
            "CompositeAlarms": [{"AlarmName": "site", "StateValue": "OK", "AlarmRule": "ALARM(a) OR ALARM(b)"}]
        });
        let alarms = alarms(&answer);
        assert_eq!(alarms.len(), 3);
        assert_eq!(alarms[0]["kind"], "metric");
        assert_eq!(alarms[0]["reason"].as_str().unwrap().len(), 200);
        assert_eq!(alarms[0]["reasonCut"], true);
        assert_eq!(
            alarms[0]["watches"],
            "AWS/Lambda Errors Sum {FunctionName=demo-fn} GreaterThanThreshold 1"
        );
        assert_eq!(alarms[1]["watches"], "AWS/Lambda Duration p99");
        assert_eq!(alarms[1]["reasonCut"], false);
        assert_eq!(alarms[2]["kind"], "composite");
        assert_eq!(alarms[2]["watches"], "ALARM(a) OR ALARM(b)");
        assert!(super::alarms(&json!({})).is_empty());
    }
}
