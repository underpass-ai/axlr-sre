//! What every use case does with an `aws` run: turn a non-zero exit into
//! the translated failure and read the JSON.

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::domain::{Failure, Invocation};
use crate::ports::AwsRunner;

/// Pages a listing walks at most before it says `more`.
pub const PAGES_MAX: usize = 20;

/// Runs and fails on any exit but zero, with the verb and the translated
/// reason and hint.
pub async fn run_json(
    runner: &dyn AwsRunner,
    invocation: &Invocation,
    profile: Option<&str>,
) -> Result<Value> {
    let completed = runner
        .run(invocation)
        .await
        .with_context(|| format!("aws {} could not run", invocation.verb()))?;
    if !completed.succeeded() {
        let failure = Failure::diagnose(&completed.stderr, profile);
        let hint = failure
            .hint
            .as_deref()
            .map(|h| format!(" ({h})"))
            .unwrap_or_default();
        return Err(anyhow!(
            "aws {}: {}{hint}",
            invocation.verb(),
            if failure.detail.is_empty() {
                format!("exit {}", completed.exit_code.unwrap_or(-1))
            } else {
                failure.detail
            }
        ));
    }
    if completed.stdout.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&completed.stdout)
        .with_context(|| format!("aws {} did not answer JSON", invocation.verb()))
}

/// The token the CLI's own paging leaves when there is more.
#[must_use]
pub fn next_token(answer: &Value) -> Option<String> {
    answer["NextToken"]
        .as_str()
        .or(answer["nextToken"].as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// A non-zero exit whose failure code is one of `codes` is not an error
/// but an absence: the policy a function does not have, for instance.
pub async fn run_json_or_absent(
    runner: &dyn AwsRunner,
    invocation: &Invocation,
    profile: Option<&str>,
    codes: &[&str],
) -> Result<Option<Value>> {
    let completed = runner
        .run(invocation)
        .await
        .with_context(|| format!("aws {} could not run", invocation.verb()))?;
    if completed.succeeded() {
        if completed.stdout.trim().is_empty() {
            return Ok(None);
        }
        return serde_json::from_str(&completed.stdout)
            .map(Some)
            .with_context(|| format!("aws {} did not answer JSON", invocation.verb()));
    }
    let failure = Failure::diagnose(&completed.stderr, profile);
    if failure
        .code
        .as_deref()
        .is_some_and(|code| codes.contains(&code))
    {
        return Ok(None);
    }
    Err(anyhow!("aws {}: {}", invocation.verb(), failure.detail))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_doubles::FakeRunner;
    use super::*;

    #[tokio::test]
    async fn a_failure_is_translated_and_a_known_code_is_an_absence() {
        let runner = FakeRunner::answering(vec![
            ("sts get-caller-identity", Ok(json!({"Account": "123456789012"}))),
            ("lambda get-policy", Err("An error occurred (ResourceNotFoundException) when calling the GetPolicy operation: The resource you requested does not exist.".into())),
            ("lambda list-aliases", Err("Unable to locate credentials".into())),
            ("lambda list-functions", Ok(json!("not json"))),
            ("logs describe-log-groups", Ok(json!(""))),
        ]);
        let identity = run_json(
            runner.as_ref(),
            &Invocation::caller_identity("eu-west-1"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(identity["Account"], "123456789012");
        let error = run_json(
            runner.as_ref(),
            &Invocation::aliases("eu-west-1", "f"),
            Some("demo"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.starts_with("aws lambda list-aliases: no AWS credentials (set AWS_MCP_PROFILE"),
            "{error}"
        );
        let error = run_json(
            runner.as_ref(),
            &Invocation::functions("eu-west-1", 10, None).unwrap(),
            None,
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("did not answer JSON"));
        let empty = run_json(
            runner.as_ref(),
            &Invocation::log_groups("eu-west-1", None, 10, None).unwrap(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(empty, Value::Null);
        let unexpected = run_json(runner.as_ref(), &Invocation::policy("eu-west-1", "f"), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            unexpected.contains("lambda get-policy: ResourceNotFoundException on GetPolicy"),
            "{unexpected}"
        );

        let absent = run_json_or_absent(
            runner.as_ref(),
            &Invocation::policy("eu-west-1", "f"),
            None,
            &["ResourceNotFoundException"],
        )
        .await
        .unwrap();
        assert_eq!(absent, None);
        let present = run_json_or_absent(
            runner.as_ref(),
            &Invocation::caller_identity("eu-west-1"),
            None,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(present.unwrap()["Account"], "123456789012");
        assert!(
            run_json_or_absent(
                runner.as_ref(),
                &Invocation::aliases("eu-west-1", "f"),
                None,
                &["X"]
            )
            .await
            .is_err()
        );
        assert!(
            run_json_or_absent(
                runner.as_ref(),
                &Invocation::functions("eu-west-1", 10, None).unwrap(),
                None,
                &[]
            )
            .await
            .is_err()
        );
        assert_eq!(
            run_json_or_absent(
                runner.as_ref(),
                &Invocation::log_groups("eu-west-1", None, 10, None).unwrap(),
                None,
                &[]
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(next_token(&json!({"NextToken": "a"})).as_deref(), Some("a"));
        assert_eq!(next_token(&json!({"nextToken": "b"})).as_deref(), Some("b"));
        assert_eq!(next_token(&json!({"NextToken": ""})), None);
        assert_eq!(runner.verbs()[0], "sts get-caller-identity");
    }

    #[tokio::test]
    async fn a_runner_that_cannot_start_is_an_error() {
        let runner = FakeRunner {
            answers: std::sync::Mutex::new(Vec::new()),
            calls: std::sync::Mutex::new(Vec::new()),
            fail_to_run: true,
        };
        let error = run_json(&runner, &Invocation::version(), None)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("could not run"));
        assert!(
            run_json_or_absent(&runner, &Invocation::version(), None, &[])
                .await
                .is_err()
        );
    }
}
