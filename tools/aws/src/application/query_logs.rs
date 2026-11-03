use std::sync::Arc;

use anyhow::Result;
use jiff::Timestamp;
use serde_json::{Value, json};

use super::shared::{PAGES_MAX, next_token, run_json};
use crate::domain::{
    EVENTS_MAX, Invocation, Regions, TimeWindow, function_name, log_events, log_group_of,
    log_groups,
};
use crate::ports::AwsRunner;

const LOG_WINDOW_DEFAULT: i64 = 30 * 60;
const EVENTS_BYTES_MAX: usize = 262_144;

/// The CloudWatch Logs reads: `logs_filter` and `logs_groups`.
pub struct QueryLogs {
    runner: Arc<dyn AwsRunner>,
    regions: Regions,
    profile: Option<String>,
}

impl QueryLogs {
    pub fn new(runner: Arc<dyn AwsRunner>, regions: Regions, profile: Option<String>) -> Self {
        Self {
            runner,
            regions,
            profile,
        }
    }

    /// The events of a log group in a window, by filter pattern, `limit` at
    /// most and 256 KiB at most. The group comes from `log_group`, or is
    /// derived from `function` as `/aws/lambda/<name>`; a derived group with
    /// no events gets a warning, since a function with its own
    /// `LoggingConfig` writes elsewhere.
    #[allow(clippy::too_many_arguments)]
    pub async fn filter(
        &self,
        region: Option<&str>,
        log_group: Option<&str>,
        function: Option<&str>,
        filter: Option<&str>,
        start: Option<&str>,
        end: Option<&str>,
        limit: i64,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let (group, derived) = match (
            log_group.map(str::trim).filter(|g| !g.is_empty()),
            function.map(str::trim).filter(|f| !f.is_empty()),
        ) {
            (Some(group), _) => (group.to_string(), false),
            (None, Some(function)) => (log_group_of(&function_name(function)?, None).0, true),
            (None, None) => {
                anyhow::bail!(crate::domain::InputError::Empty("log_group or function"))
            }
        };
        if !(1..=EVENTS_MAX).contains(&limit) {
            anyhow::bail!(crate::domain::InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: EVENTS_MAX
            });
        }
        let window = TimeWindow::parse(start, end, Timestamp::now(), LOG_WINDOW_DEFAULT)?;
        let wanted = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut events: Vec<Value> = Vec::new();
        let mut bytes = 0usize;
        let mut token: Option<String> = None;
        let mut truncated = false;
        for _ in 0..PAGES_MAX {
            let page_size = i64::try_from(wanted - events.len())
                .unwrap_or(EVENTS_MAX)
                .min(EVENTS_MAX);
            let invocation = Invocation::log_events(
                &region,
                &group,
                &window,
                filter,
                page_size,
                token.as_deref(),
            )?;
            let answer =
                run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
            for event in log_events(&answer) {
                let size = event["message"].as_str().map_or(0, str::len);
                if events.len() == wanted || bytes + size > EVENTS_BYTES_MAX {
                    truncated = true;
                    break;
                }
                bytes += size;
                events.push(event);
            }
            token = next_token(&answer);
            if truncated || token.is_none() {
                break;
            }
            if events.len() >= wanted {
                truncated = true;
                break;
            }
        }
        if token.is_some() {
            truncated = true;
        }
        let mut report = json!({
            "region": region,
            "logGroup": group,
            "logGroupDerived": derived,
            "filter": filter,
            "start": window.start_iso(),
            "end": window.end_iso(),
            "count": events.len(),
            "truncated": truncated,
            "events": events,
        });
        if derived && report["count"] == 0 {
            report["warning"] = json!(format!(
                "no events in `{group}`, which was derived from the function name; a function with its own LoggingConfig writes elsewhere: lambda_function shows its logGroup, pass it as log_group"
            ));
        }
        Ok(report)
    }

    /// The log groups of a region by name prefix, `limit` at most.
    pub async fn groups(
        &self,
        region: Option<&str>,
        prefix: Option<&str>,
        limit: i64,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let invocation = Invocation::log_groups(&region, prefix, limit, None)?;
        let answer = run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
        let groups = log_groups(&answer);
        Ok(json!({
            "region": region,
            "prefix": prefix,
            "count": groups.len(),
            "more": next_token(&answer).is_some(),
            "groups": groups,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeRunner;
    use super::*;

    fn logs(runner: &Arc<FakeRunner>) -> QueryLogs {
        let runner: Arc<dyn AwsRunner> = runner.clone();
        QueryLogs::new(runner, Regions::new(Some("eu-west-1")).unwrap(), None)
    }

    fn page(messages: &[&str], next: Option<&str>) -> Value {
        let mut v = json!({"events": messages.iter().enumerate().map(|(i, m)| json!({"timestamp": 1793707200000i64 + i as i64 * 1000, "logStreamName": "s", "message": m})).collect::<Vec<_>>()});
        if let Some(next) = next {
            v["nextToken"] = json!(next);
        }
        v
    }

    #[tokio::test]
    async fn events_are_paged_to_the_limit_and_a_derived_group_without_events_warns() {
        let runner = FakeRunner::answering(vec![
            ("--starting-token t1", Ok(page(&["c", "d"], None))),
            ("logs filter-log-events", Ok(page(&["a", "b"], Some("t1")))),
        ]);
        let all = logs(&runner)
            .filter(
                None,
                Some("/aws/lambda/demo"),
                None,
                Some("ERROR"),
                Some("1h"),
                None,
                10,
            )
            .await
            .unwrap();
        assert_eq!(all["count"], 4);
        assert_eq!(all["truncated"], false);
        assert_eq!(all["logGroupDerived"], false);
        assert_eq!(all["events"][0]["message"], "a");
        assert_eq!(all["warning"], Value::Null);
        assert!(runner.lines()[0].contains("--filter-pattern ERROR"));
        assert!(runner.lines()[0].contains("--max-items 10"));
        let bounded = logs(&runner)
            .filter(None, None, Some("demo"), None, None, None, 3)
            .await
            .unwrap();
        assert_eq!(bounded["count"], 3);
        assert_eq!(bounded["truncated"], true);
        runner.calls.lock().unwrap().clear();
        let exact = logs(&runner)
            .filter(None, Some("/g"), None, None, None, None, 2)
            .await
            .unwrap();
        assert_eq!(exact["count"], 2);
        assert_eq!(
            exact["truncated"], true,
            "a token was left after a full page"
        );
        assert_eq!(runner.verbs().len(), 1, "no second request for zero events");
        assert_eq!(bounded["logGroup"], "/aws/lambda/demo");
        assert_eq!(bounded["logGroupDerived"], true);
        runner.set(vec![("logs filter-log-events", Ok(json!({"events": []})))]);
        let empty = logs(&runner)
            .filter(None, None, Some("demo"), None, None, None, 10)
            .await
            .unwrap();
        assert!(empty["warning"].as_str().unwrap().contains("LoggingConfig"));
        runner.set(vec![(
            "logs filter-log-events",
            Ok(page(&["x"], Some("loop"))),
        )]);
        let looping = logs(&runner)
            .filter(None, Some("/g"), None, None, None, None, 1000)
            .await
            .unwrap();
        assert_eq!(looping["count"], PAGES_MAX);
        assert_eq!(looping["truncated"], true);
        let big: Vec<String> = vec!["m".repeat(4000); 80];
        let refs: Vec<&str> = big.iter().map(String::as_str).collect();
        runner.set(vec![("logs filter-log-events", Ok(page(&refs, None)))]);
        let heavy = logs(&runner)
            .filter(None, Some("/g"), None, None, None, None, 1000)
            .await
            .unwrap();
        assert_eq!(heavy["truncated"], true);
        assert!(heavy["count"].as_u64().unwrap() < 80);
    }

    #[tokio::test]
    async fn bad_log_input_fails_before_any_run() {
        let runner = FakeRunner::answering(vec![]);
        let logs = logs(&runner);
        assert!(
            logs.filter(None, None, None, None, None, None, 10)
                .await
                .is_err()
        );
        assert!(
            logs.filter(None, Some("bad group"), None, None, None, None, 10)
                .await
                .is_err()
        );
        assert!(
            logs.filter(None, None, Some("-f"), None, None, None, 10)
                .await
                .is_err()
        );
        assert!(
            logs.filter(None, Some("/g"), None, None, None, None, 0)
                .await
                .is_err()
        );
        assert!(
            logs.filter(None, Some("/g"), None, None, Some("x"), None, 10)
                .await
                .is_err()
        );
        assert!(
            logs.filter(Some("mars"), Some("/g"), None, None, None, None, 10)
                .await
                .is_err()
        );
        assert!(logs.groups(None, Some("a b"), 10).await.is_err());
        assert!(logs.groups(None, None, 0).await.is_err());
        assert!(runner.verbs().is_empty());
    }

    #[tokio::test]
    async fn groups_are_listed_with_a_more_flag() {
        let runner = FakeRunner::answering(vec![(
            "logs describe-log-groups",
            Ok(json!({"logGroups": [{"logGroupName": "/aws/lambda/a"}], "NextToken": "t"})),
        )]);
        let answer = logs(&runner)
            .groups(None, Some("/aws/lambda/"), 1)
            .await
            .unwrap();
        assert_eq!(answer["count"], 1);
        assert_eq!(answer["more"], true);
        assert_eq!(answer["groups"][0]["name"], "/aws/lambda/a");
        assert!(runner.lines()[0].contains("--log-group-name-prefix /aws/lambda/"));
    }
}
