use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{next_token, run_json};
use crate::domain::{Invocation, ServerConfig, alarms};
use crate::ports::AwsRunner;

const REGIONS_MAX: usize = 5;
const FUNCTIONS_PAGE: i64 = 50;

/// `discover`: who the CLI is, the regions in scope and, for each of the
/// first five, how many alarms are in ALARM and one page of functions.
/// With no allowlist, `region` names the one to look at.
pub struct DiscoverScope {
    runner: Arc<dyn AwsRunner>,
    config: ServerConfig,
}

impl DiscoverScope {
    pub fn new(runner: Arc<dyn AwsRunner>, config: ServerConfig) -> Self {
        Self { runner, config }
    }

    pub async fn execute(&self, region: Option<&str>) -> Result<Value> {
        let regions: Vec<String> = match region {
            Some(named) => vec![self.config.regions().resolve(Some(named))?],
            None if self.config.regions().allowed().is_empty() => {
                anyhow::bail!(crate::domain::InputError::RegionRequired)
            }
            None => self.config.regions().allowed().to_vec(),
        };
        let profile = self.config.profile();
        let identity = run_json(
            self.runner.as_ref(),
            &Invocation::caller_identity(&regions[0]),
            profile,
        )
        .await?;
        let mut per_region = Vec::new();
        for region in regions.iter().take(REGIONS_MAX) {
            let in_alarm = run_json(
                self.runner.as_ref(),
                &Invocation::alarms(region, Some("ALARM"), None, 100, None)?,
                profile,
            )
            .await
            .map(|a| json!({"count": alarms(&a).len(), "more": next_token(&a).is_some()}))
            .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
            let functions = run_json(
                self.runner.as_ref(),
                &Invocation::functions(region, FUNCTIONS_PAGE, None)?,
                profile,
            )
            .await
            .map(|f| {
                json!({
                    "count": f["Functions"].as_array().map_or(0, Vec::len),
                    "more": next_token(&f).is_some(),
                })
            })
            .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
            per_region
                .push(json!({"region": region, "alarmsInAlarm": in_alarm, "functions": functions}));
        }
        Ok(json!({
            "identity": {"account": identity["Account"], "arn": identity["Arn"]},
            "profile": profile,
            "regions": regions,
            "regionsShown": per_region.len(),
            "perRegion": per_region,
            "reads": ["metric_query", "lambda_health", "alarms", "lambda_functions", "lambda_function", "logs_filter", "logs_groups"],
            "writes": [],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeRunner;
    use super::*;

    #[tokio::test]
    async fn the_scope_counts_alarms_and_functions_per_region() {
        let runner = FakeRunner::answering(vec![
            (
                "sts get-caller-identity",
                Ok(json!({"Account": "123456789012", "Arn": "arn:x"})),
            ),
            (
                "cloudwatch describe-alarms",
                Ok(json!({"MetricAlarms": [{"AlarmName": "a", "StateValue": "ALARM"}]})),
            ),
            (
                "lambda list-functions",
                Ok(json!({"Functions": [{"FunctionName": "f"}], "NextToken": "t"})),
            ),
        ]);
        let discover = DiscoverScope::new(
            runner.clone(),
            ServerConfig::new(Some("eu-west-1,us-east-1"), None, None).unwrap(),
        );
        let answer = discover.execute(None).await.unwrap();
        assert_eq!(answer["identity"]["account"], "123456789012");
        assert_eq!(answer["regions"].as_array().unwrap().len(), 2);
        assert_eq!(answer["perRegion"][0]["alarmsInAlarm"]["count"], 1);
        assert_eq!(answer["perRegion"][1]["functions"]["more"], true);
        assert_eq!(answer["writes"], json!([]));
        assert_eq!(runner.verbs().len(), 5);
        let one = discover.execute(Some("us-east-1")).await.unwrap();
        assert_eq!(one["regions"], json!(["us-east-1"]));
        assert!(discover.execute(Some("ap-south-1")).await.is_err());

        runner.set(vec![
            ("sts get-caller-identity", Ok(json!({"Account": "123456789012"}))),
            ("cloudwatch describe-alarms", Err("An error occurred (AccessDeniedException) when calling the DescribeAlarms operation: no".into())),
            ("lambda list-functions", Err("An error occurred (AccessDeniedException) when calling the ListFunctions operation: no".into())),
        ]);
        let denied = discover.execute(None).await.unwrap();
        assert!(
            denied["perRegion"][0]["alarmsInAlarm"]["error"]
                .as_str()
                .unwrap()
                .contains("AccessDenied")
        );
        assert!(
            denied["perRegion"][0]["functions"]["error"]
                .as_str()
                .unwrap()
                .contains("ListFunctions")
        );
        let any = DiscoverScope::new(runner.clone(), ServerConfig::new(None, None, None).unwrap());
        assert!(any.execute(None).await.is_err());
        assert_eq!(
            any.execute(Some("sa-east-1")).await.unwrap()["regionsShown"],
            1
        );
        runner.set(vec![(
            "sts get-caller-identity",
            Err("Unable to locate credentials".into()),
        )]);
        assert!(discover.execute(None).await.is_err());
    }
}
