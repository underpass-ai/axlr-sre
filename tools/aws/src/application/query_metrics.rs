use std::sync::Arc;

use anyhow::Result;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::shared::{PAGES_MAX, next_token, run_json};
use crate::domain::{
    HEALTH_METRICS, Invocation, MetricQuery, Regions, TimeWindow, alarms, function_name, series_of,
};
use crate::ports::AwsRunner;

const METRIC_WINDOW_DEFAULT: i64 = 3 * 3600;

/// The CloudWatch reads: `metric_query`, `lambda_health`, `alarms`.
pub struct QueryMetrics {
    runner: Arc<dyn AwsRunner>,
    regions: Regions,
    profile: Option<String>,
}

impl QueryMetrics {
    pub fn new(runner: Arc<dyn AwsRunner>, regions: Regions, profile: Option<String>) -> Self {
        Self {
            runner,
            regions,
            profile,
        }
    }

    /// One metric's series over a window, compacted; zeros kept, since the
    /// caller chose the metric.
    #[allow(clippy::too_many_arguments)]
    pub async fn metric_query(
        &self,
        region: Option<&str>,
        namespace: &str,
        metric: &str,
        dimensions: &Map<String, Value>,
        stat: &str,
        period: i64,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let dimensions: Vec<(String, String)> = dimensions
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect();
        let query = MetricQuery::new("m", namespace, metric, &dimensions, stat, period)?;
        let window = TimeWindow::parse(start, end, Timestamp::now(), METRIC_WINDOW_DEFAULT)?;
        let invocation = Invocation::metric_data(&region, &[query], &window)?;
        let answer = run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
        let series = series_of(&answer, "m");
        let mut report = json!({
            "region": region,
            "namespace": namespace,
            "metric": metric,
            "stat": stat,
            "period": period,
            "start": window.start_iso(),
            "end": window.end_iso(),
            "points": series.points().len(),
            "last": series.last_value(),
            "series": series.compact(true),
        });
        mark_truncated(&mut report, &answer);
        Ok(report)
    }

    /// The six metrics of a function in one call. A series that is all
    /// zero is summarised; no invocations at all is said before the series.
    pub async fn lambda_health(
        &self,
        region: Option<&str>,
        function: &str,
        period: i64,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let function = function_name(function)?;
        let dimensions = [("FunctionName".to_string(), function.clone())];
        let queries: Vec<MetricQuery> = HEALTH_METRICS
            .iter()
            .map(|(id, metric, stat)| {
                MetricQuery::new(id, "AWS/Lambda", metric, &dimensions, stat, period)
            })
            .collect::<Result<_, _>>()?;
        let window = TimeWindow::parse(start, end, Timestamp::now(), METRIC_WINDOW_DEFAULT)?;
        let invocation = Invocation::metric_data(&region, &queries, &window)?;
        let answer = run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
        let mut report = json!({
            "region": region,
            "function": function,
            "period": period,
            "start": window.start_iso(),
            "end": window.end_iso(),
        });
        let invocations = series_of(&answer, "invocations");
        let errors = series_of(&answer, "errors");
        if invocations.is_empty() {
            report["warning"] = json!(if errors.is_empty() {
                format!(
                    "CloudWatch has not one point of `{function}` in this window: it was not invoked, or the name is not the exact function name. This is not a failure of the function"
                )
            } else {
                format!("`{function}` has no invocations in the window but other metrics do")
            });
        }
        for (id, _, _) in HEALTH_METRICS {
            report[id] = series_of(&answer, id).compact(false);
        }
        report["errorsTotal"] = json!(errors.points().iter().map(|(_, v)| v).sum::<f64>());
        report["invocationsTotal"] =
            json!(invocations.points().iter().map(|(_, v)| v).sum::<f64>());
        mark_truncated(&mut report, &answer);
        Ok(report)
    }

    /// The alarms of a region, by state and name prefix, `limit` at most.
    pub async fn alarms(
        &self,
        region: Option<&str>,
        state: Option<&str>,
        prefix: Option<&str>,
        limit: i64,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let mut found: Vec<Value> = Vec::new();
        let mut token: Option<String> = None;
        let mut more = false;
        for _ in 0..PAGES_MAX {
            let invocation = Invocation::alarms(&region, state, prefix, limit, token.as_deref())?;
            let answer =
                run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
            found.extend(alarms(&answer));
            token = next_token(&answer);
            if found.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
                more =
                    token.is_some() || found.len() > usize::try_from(limit).unwrap_or(usize::MAX);
                found.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                break;
            }
            if token.is_none() {
                break;
            }
        }
        if token.is_some() {
            more = true;
        }
        let in_alarm = found.iter().filter(|a| a["state"] == "ALARM").count();
        Ok(json!({
            "region": region,
            "state": state,
            "count": found.len(),
            "inAlarm": in_alarm,
            "more": more,
            "alarms": found,
        }))
    }
}

/// CloudWatch answers 100 800 points at most per call and leaves a token
/// for the rest; since only one page is read, the answer must say so.
fn mark_truncated(report: &mut Value, answer: &Value) {
    if next_token(answer).is_some() {
        report["truncated"] = json!(true);
        report["hint"] = json!(
            "CloudWatch had more points than one answer carries: raise period or shorten the window"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeRunner;
    use super::*;

    fn metrics(runner: &Arc<FakeRunner>, regions: Option<&str>) -> QueryMetrics {
        let runner: Arc<dyn AwsRunner> = runner.clone();
        QueryMetrics::new(runner, Regions::new(regions).unwrap(), Some("demo".into()))
    }

    fn data(id: &str, values: &[f64]) -> Value {
        json!({"Id": id, "Timestamps": values.iter().enumerate().map(|(i, _)| format!("2026-11-03T09:{:02}:00+00:00", i)).collect::<Vec<_>>(), "Values": values})
    }

    #[tokio::test]
    async fn a_metric_query_compacts_its_series_and_keeps_zeros() {
        let runner = FakeRunner::answering(vec![(
            "cloudwatch get-metric-data",
            Ok(json!({"MetricDataResults": [data("m", &[0.0, 0.0, 3.0])]})),
        )]);
        let answer = metrics(&runner, Some("eu-west-1"))
            .metric_query(
                None,
                "AWS/SQS",
                "ApproximateNumberOfMessagesVisible",
                &serde_json::from_value(json!({"QueueName": "demo-q"})).unwrap(),
                "Maximum",
                60,
                Some("1h"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(answer["region"], "eu-west-1");
        assert_eq!(answer["points"], 3);
        assert_eq!(answer["last"], 3.0);
        assert_eq!(answer["series"].as_array().unwrap().len(), 2);
        assert!(runner.lines()[0].contains("\"QueueName\",\"Value\":\"demo-q\""));
        let zeros = FakeRunner::answering(vec![(
            "cloudwatch get-metric-data",
            Ok(json!({"MetricDataResults": [data("m", &[0.0, 0.0])]})),
        )]);
        let answer = metrics(&zeros, Some("eu-west-1"))
            .metric_query(None, "AWS/SQS", "V", &Map::new(), "Sum", 300, None, None)
            .await
            .unwrap();
        assert_eq!(answer["series"].as_array().unwrap().len(), 2);
        assert_eq!(answer["truncated"], Value::Null);
        zeros.set(vec![(
            "cloudwatch get-metric-data",
            Ok(json!({"MetricDataResults": [data("m", &[1.0])], "NextToken": "more"})),
        )]);
        let cut = metrics(&zeros, Some("eu-west-1"))
            .metric_query(None, "AWS/SQS", "V", &Map::new(), "Sum", 60, None, None)
            .await
            .unwrap();
        assert_eq!(cut["truncated"], true);
        assert!(cut["hint"].as_str().unwrap().contains("raise period"));
        let health = metrics(&zeros, Some("eu-west-1"))
            .lambda_health(None, "f", 60, None, None)
            .await
            .unwrap();
        assert_eq!(health["truncated"], true);
    }

    #[tokio::test]
    async fn bad_metric_input_fails_before_any_run() {
        let runner = FakeRunner::answering(vec![]);
        let metrics = metrics(&runner, None);
        for (region, ns, metric, stat, period, start) in [
            (None, "AWS/SQS", "V", "Sum", 300, None),
            (Some("eu-west-1"), "", "V", "Sum", 300, None),
            (Some("eu-west-1"), "AWS/SQS", "V", "median", 300, None),
            (Some("eu-west-1"), "AWS/SQS", "V", "Sum", 7, None),
            (Some("eu-west-1"), "AWS/SQS", "V", "Sum", 60, Some("never")),
        ] {
            assert!(
                metrics
                    .metric_query(region, ns, metric, &Map::new(), stat, period, start, None)
                    .await
                    .is_err()
            );
        }
        assert!(
            metrics
                .lambda_health(Some("eu-west-1"), "", 60, None, None)
                .await
                .is_err()
        );
        assert!(
            metrics
                .lambda_health(None, "f", 60, None, None)
                .await
                .is_err()
        );
        assert!(
            metrics
                .alarms(Some("eu-west-1"), Some("red"), None, 10)
                .await
                .is_err()
        );
        assert!(
            metrics
                .alarms(Some("eu-west-1"), None, None, 0)
                .await
                .is_err()
        );
        assert!(runner.verbs().is_empty());
    }

    #[tokio::test]
    async fn lambda_health_asks_six_metrics_and_warns_without_invocations() {
        let runner = FakeRunner::answering(vec![(
            "cloudwatch get-metric-data",
            Ok(
                json!({"MetricDataResults": [data("invocations", &[10.0, 12.0]), data("errors", &[0.0, 0.0]), data("duration_p99_ms", &[120.5, 130.25])]}),
            ),
        )]);
        let health = metrics(&runner, Some("eu-west-1"))
            .lambda_health(None, "demo-fn", 300, None, None)
            .await
            .unwrap();
        assert_eq!(health["function"], "demo-fn");
        assert_eq!(health["warning"], Value::Null);
        assert_eq!(health["invocationsTotal"], 22.0);
        assert_eq!(health["errorsTotal"], 0.0);
        assert_eq!(health["errors"], "all zero (2 points)");
        assert_eq!(health["throttles"], "no data");
        assert_eq!(health["duration_p99_ms"].as_array().unwrap().len(), 2);
        let line = &runner.lines()[0];
        assert!(line.contains("\"Id\":\"concurrent_max\"") && line.contains("\"Stat\":\"p99\""));

        runner.set(vec![(
            "cloudwatch get-metric-data",
            Ok(json!({"MetricDataResults": []})),
        )]);
        let quiet = metrics(&runner, Some("eu-west-1"))
            .lambda_health(None, "demo-fn", 300, None, None)
            .await
            .unwrap();
        assert!(quiet["warning"].as_str().unwrap().contains("not a failure"));
        runner.set(vec![(
            "cloudwatch get-metric-data",
            Ok(json!({"MetricDataResults": [data("errors", &[1.0])]})),
        )]);
        let odd = metrics(&runner, Some("eu-west-1"))
            .lambda_health(None, "demo-fn", 300, None, None)
            .await
            .unwrap();
        assert!(
            odd["warning"]
                .as_str()
                .unwrap()
                .contains("other metrics do")
        );
        runner.set(vec![("cloudwatch get-metric-data", Err("An error occurred (AccessDeniedException) when calling the GetMetricData operation: no".into()))]);
        let error = metrics(&runner, Some("eu-west-1"))
            .lambda_health(None, "demo-fn", 300, None, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("AccessDeniedException") && error.contains("doctor"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn alarms_are_paged_up_to_the_limit() {
        let page = |names: &[&str], next: Option<&str>| {
            let mut v = json!({"MetricAlarms": names.iter().map(|n| json!({"AlarmName": n, "StateValue": "ALARM"})).collect::<Vec<_>>()});
            if let Some(next) = next {
                v["NextToken"] = json!(next);
            }
            v
        };
        let runner = FakeRunner::answering(vec![
            ("--starting-token t1", Ok(page(&["c"], None))),
            (
                "cloudwatch describe-alarms",
                Ok(page(&["a", "b"], Some("t1"))),
            ),
        ]);
        let answer = metrics(&runner, Some("eu-west-1"))
            .alarms(None, Some("alarm"), Some("demo"), 10)
            .await
            .unwrap();
        assert_eq!(answer["count"], 3);
        assert_eq!(answer["inAlarm"], 3);
        assert_eq!(answer["more"], false);
        assert_eq!(runner.verbs().len(), 2);
        let answer = metrics(&runner, Some("eu-west-1"))
            .alarms(None, None, None, 2)
            .await
            .unwrap();
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["more"], true);
        runner.set(vec![(
            "cloudwatch describe-alarms",
            Ok(page(&["a"], Some("loop"))),
        )]);
        let answer = metrics(&runner, Some("eu-west-1"))
            .alarms(None, None, None, 100)
            .await
            .unwrap();
        assert_eq!(answer["count"], PAGES_MAX);
        assert_eq!(answer["more"], true);
    }
}
