use std::sync::Arc;

use anyhow::{Result, anyhow};
use aws_mcp::application::shared::run_json;
use aws_mcp::ports::AwsRunner;

use crate::domain::{Call, QueueFacts, QueueRef};

/// What every use case needs to know about a queue before acting: its URL
/// (a name is resolved in the region) and its facts, sources included.
pub struct QueueLookup {
    runner: Arc<dyn AwsRunner>,
    profile: Option<String>,
}

impl QueueLookup {
    pub fn new(runner: Arc<dyn AwsRunner>, profile: Option<String>) -> Self {
        Self { runner, profile }
    }

    #[must_use]
    pub fn runner(&self) -> &Arc<dyn AwsRunner> {
        &self.runner
    }

    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    pub async fn url(&self, region: &str, raw: &str) -> Result<String> {
        match QueueRef::parse(raw, region)? {
            QueueRef::Url(url) => Ok(url),
            QueueRef::Name(name) => {
                let answer = run_json(
                    self.runner.as_ref(),
                    &Call::queue_url(region, &name)?,
                    self.profile(),
                )
                .await?;
                answer["QueueUrl"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("get-queue-url answered without a QueueUrl for {name}"))
            }
        }
    }

    /// The attributes alone, one run: no sources, so `is_dead_letter` is
    /// not answered here.
    pub async fn attributes(&self, region: &str, url: &str) -> Result<QueueFacts> {
        let attributes = run_json(
            self.runner.as_ref(),
            &Call::attributes(region, url)?,
            self.profile(),
        )
        .await?;
        QueueFacts::read(url, &attributes)
            .ok_or_else(|| anyhow!("get-queue-attributes answered without attributes for {url}"))
    }

    pub async fn facts(&self, region: &str, url: &str) -> Result<QueueFacts> {
        let mut facts = self.attributes(region, url).await?;
        let sources = run_json(
            self.runner.as_ref(),
            &Call::dead_letter_sources(region, url)?,
            self.profile(),
        )
        .await?;
        facts.sources = sources["queueUrls"]
            .as_array()
            .map(|urls| {
                urls.iter()
                    .filter_map(|u| u.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(facts)
    }

    /// Both at once, for the calls that take a queue by URL or name.
    pub async fn resolve(&self, region: &str, raw: &str) -> Result<QueueFacts> {
        let url = self.url(region, raw).await?;
        self.facts(region, &url).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_doubles::{FakeRunner, SOURCE, URL, dlq_attributes};
    use super::*;

    #[tokio::test]
    async fn a_name_is_resolved_and_the_facts_carry_the_sources() {
        let runner = FakeRunner::answering(vec![
            ("sqs get-queue-url", Ok(json!({"QueueUrl": URL}))),
            ("sqs get-queue-attributes", Ok(dlq_attributes(3))),
            (
                "sqs list-dead-letter-source-queues",
                Ok(json!({"queueUrls": [SOURCE]})),
            ),
        ]);
        let lookup = QueueLookup::new(runner.clone(), Some("demo".into()));
        let facts = lookup
            .resolve("eu-west-1", "demo-orders-dlq")
            .await
            .unwrap();
        assert_eq!(facts.url, URL);
        assert_eq!(facts.depth.visible, 3);
        assert_eq!(facts.sources, [SOURCE]);
        assert!(facts.is_dead_letter());
        assert_eq!(
            runner.verbs(),
            [
                "sqs get-queue-url",
                "sqs get-queue-attributes",
                "sqs list-dead-letter-source-queues"
            ]
        );
        assert_eq!(lookup.profile(), Some("demo"));
        assert_eq!(lookup.url("eu-west-1", URL).await.unwrap(), URL);
        assert!(lookup.url("us-east-1", URL).await.is_err());
        runner.set(vec![
            ("sqs get-queue-url", Ok(json!({}))),
            ("sqs get-queue-attributes", Ok(json!({}))),
        ]);
        assert!(
            lookup
                .url("eu-west-1", "x")
                .await
                .unwrap_err()
                .to_string()
                .contains("without a QueueUrl")
        );
        assert!(
            lookup
                .facts("eu-west-1", URL)
                .await
                .unwrap_err()
                .to_string()
                .contains("without attributes")
        );
        runner.set(vec![("sqs get-queue-url", Err("An error occurred (AWS.SimpleQueueService.NonExistentQueue) when calling the GetQueueUrl operation: The specified queue does not exist.".into()))]);
        let error = lookup
            .url("eu-west-1", "ghost")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("NonExistentQueue"), "{error}");
    }
}
