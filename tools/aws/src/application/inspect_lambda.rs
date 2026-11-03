use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{PAGES_MAX, next_token, run_json, run_json_or_absent};
use crate::domain::{
    FUNCTIONS_MAX, Invocation, Regions, function_detail, function_name, function_summary,
};
use crate::ports::AwsRunner;

/// The Lambda reads: `lambda_functions` and `lambda_function`.
pub struct InspectLambda {
    runner: Arc<dyn AwsRunner>,
    regions: Regions,
    profile: Option<String>,
}

impl InspectLambda {
    pub fn new(runner: Arc<dyn AwsRunner>, regions: Regions, profile: Option<String>) -> Self {
        Self {
            runner,
            regions,
            profile,
        }
    }

    /// The functions of a region whose name contains `contains`, sorted by
    /// name, `limit` at most. The name filter runs here, after each page:
    /// the API has none.
    pub async fn functions(
        &self,
        region: Option<&str>,
        contains: Option<&str>,
        limit: i64,
    ) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        if !(1..=FUNCTIONS_MAX).contains(&limit) {
            anyhow::bail!(crate::domain::InputError::OutOfRange {
                what: "limit",
                min: 1,
                max: FUNCTIONS_MAX
            });
        }
        let needle = contains
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_ascii_lowercase);
        let wanted = usize::try_from(limit).unwrap_or(usize::MAX);
        let mut found: Vec<Value> = Vec::new();
        let mut token: Option<String> = None;
        let mut more = false;
        for _ in 0..PAGES_MAX {
            let invocation = Invocation::functions(&region, FUNCTIONS_MAX, token.as_deref())?;
            let answer =
                run_json(self.runner.as_ref(), &invocation, self.profile.as_deref()).await?;
            for raw in answer["Functions"].as_array().cloned().unwrap_or_default() {
                let name = raw["FunctionName"].as_str().unwrap_or_default();
                let matches = needle
                    .as_deref()
                    .is_none_or(|n| name.to_ascii_lowercase().contains(n));
                if matches {
                    if found.len() == wanted {
                        more = true;
                        break;
                    }
                    found.push(function_summary(&raw));
                }
            }
            token = next_token(&answer);
            if more || token.is_none() {
                break;
            }
        }
        if token.is_some() {
            more = true;
        }
        found.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(json!({
            "region": region,
            "contains": needle,
            "count": found.len(),
            "more": more,
            "functions": found,
        }))
    }

    /// One function with its aliases, triggers and log group. A function
    /// without a resource policy has no push triggers, which is not an error.
    pub async fn function(&self, region: Option<&str>, function: &str) -> Result<Value> {
        let region = self.regions.resolve(region)?;
        let function = function_name(function)?;
        let profile = self.profile.as_deref();
        let configuration = run_json(
            self.runner.as_ref(),
            &Invocation::function_configuration(&region, &function),
            profile,
        )
        .await?;
        let aliases = run_json(
            self.runner.as_ref(),
            &Invocation::aliases(&region, &function),
            profile,
        )
        .await?;
        let mappings = run_json(
            self.runner.as_ref(),
            &Invocation::event_source_mappings(&region, &function),
            profile,
        )
        .await?;
        let policy = run_json_or_absent(
            self.runner.as_ref(),
            &Invocation::policy(&region, &function),
            profile,
            &["ResourceNotFoundException"],
        )
        .await?;
        let mut detail = function_detail(
            &configuration,
            Some(&aliases),
            Some(&mappings),
            policy.as_ref(),
        );
        detail["region"] = json!(region);
        Ok(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::FakeRunner;
    use super::*;

    fn lambda(runner: &Arc<FakeRunner>) -> InspectLambda {
        let runner: Arc<dyn AwsRunner> = runner.clone();
        InspectLambda::new(runner, Regions::new(Some("eu-west-1")).unwrap(), None)
    }

    fn function(name: &str) -> Value {
        json!({"FunctionName": name, "Runtime": "python3.12", "MemorySize": 128, "Timeout": 3, "Environment": {"Variables": {"SECRET": "s"}}})
    }

    #[tokio::test]
    async fn functions_are_filtered_sorted_and_bounded() {
        let runner = FakeRunner::answering(vec![
            (
                "--starting-token t1",
                Ok(json!({"Functions": [function("zeta-api"), function("alpha-worker")]})),
            ),
            (
                "lambda list-functions",
                Ok(
                    json!({"Functions": [function("demo-api"), function("other")], "NextToken": "t1"}),
                ),
            ),
        ]);
        let answer = lambda(&runner)
            .functions(None, Some("API"), 10)
            .await
            .unwrap();
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["functions"][0]["name"], "demo-api");
        assert_eq!(answer["functions"][1]["name"], "zeta-api");
        assert_eq!(answer["more"], false);
        assert!(!answer.to_string().contains("SECRET"));
        let bounded = lambda(&runner).functions(None, None, 1).await.unwrap();
        assert_eq!(bounded["count"], 1);
        assert_eq!(bounded["more"], true);
        assert!(lambda(&runner).functions(None, None, 0).await.is_err());
        assert!(
            lambda(&runner)
                .functions(Some("us-east-1"), None, 5)
                .await
                .is_err()
        );
        runner.set(vec![(
            "lambda list-functions",
            Ok(json!({"Functions": [function("a")], "NextToken": "loop"})),
        )]);
        let looping = lambda(&runner).functions(None, None, 100).await.unwrap();
        assert_eq!(looping["count"], PAGES_MAX);
        assert_eq!(looping["more"], true);
    }

    #[tokio::test]
    async fn one_function_gathers_configuration_aliases_mappings_and_policy() {
        let runner = FakeRunner::answering(vec![
            ("lambda get-function-configuration", Ok(json!({"FunctionName": "demo-fn", "FunctionArn": "arn:f", "Handler": "h", "LoggingConfig": {"LogGroup": "/custom"}}))),
            ("lambda list-aliases", Ok(json!({"Aliases": [{"Name": "live", "FunctionVersion": "3"}]}))),
            ("lambda list-event-source-mappings", Ok(json!({"EventSourceMappings": [{"EventSourceArn": "arn:sqs", "State": "Enabled"}]}))),
            ("lambda get-policy", Err("An error occurred (ResourceNotFoundException) when calling the GetPolicy operation: none".into())),
        ]);
        let detail = lambda(&runner).function(None, "demo-fn").await.unwrap();
        assert_eq!(detail["region"], "eu-west-1");
        assert_eq!(detail["logGroup"], "/custom");
        assert_eq!(detail["aliases"][0]["name"], "live");
        assert_eq!(detail["triggers"].as_array().unwrap().len(), 1);
        assert_eq!(runner.verbs().len(), 4);
        assert!(lambda(&runner).function(None, "").await.is_err());
        runner.set(vec![("lambda get-function-configuration", Err("An error occurred (ResourceNotFoundException) when calling the GetFunctionConfiguration operation: Function not found".into()))]);
        let error = lambda(&runner)
            .function(None, "ghost")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Function not found") && error.contains("exact name"),
            "{error}"
        );
    }
}
