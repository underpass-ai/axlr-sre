use std::sync::Arc;

use anyhow::Result;
use aws_mcp::application::shared::{next_token, run_json};
use aws_mcp::domain::Invocation;
use serde_json::{Value, json};

use super::QueueLookup;
use crate::domain::{Call, MessagingConfig, queue_name_of};

const REGIONS_MAX: usize = 5;
const PAGE: i64 = 50;

/// `discover`: who the CLI is, the regions in scope and, for each of the
/// first five, one page of queue names and one of topics. Depth is not
/// read here: `queues` with `with_depth` does that.
pub struct DiscoverScope {
    lookup: Arc<QueueLookup>,
    config: MessagingConfig,
}

impl DiscoverScope {
    pub fn new(lookup: Arc<QueueLookup>, config: MessagingConfig) -> Self {
        Self { lookup, config }
    }

    pub async fn execute(&self, region: Option<&str>) -> Result<Value> {
        let regions: Vec<String> = match region {
            Some(named) => vec![self.config.aws().regions().resolve(Some(named))?],
            None if self.config.aws().regions().allowed().is_empty() => {
                anyhow::bail!(aws_mcp::domain::InputError::RegionRequired)
            }
            None => self.config.aws().regions().allowed().to_vec(),
        };
        let identity = run_json(
            self.lookup.runner().as_ref(),
            &Invocation::caller_identity(&regions[0]),
            self.lookup.profile(),
        )
        .await?;
        let mut per_region = Vec::new();
        for region in regions.iter().take(REGIONS_MAX) {
            let queues = run_json(
                self.lookup.runner().as_ref(),
                &Call::list_queues(region, None, PAGE, None)?,
                self.lookup.profile(),
            )
            .await
            .map(|a| {
                let names: Vec<String> = a["QueueUrls"]
                    .as_array()
                    .map(|urls| urls.iter().filter_map(|u| u.as_str()).map(queue_name_of).collect())
                    .unwrap_or_default();
                json!({"count": names.len(), "more": next_token(&a).is_some(), "deadLetterByName": names.iter().filter(|n| n.to_ascii_lowercase().contains("dlq") || n.to_ascii_lowercase().contains("dead")).count(), "names": names})
            })
            .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
            let topics = run_json(
                self.lookup.runner().as_ref(),
                &Call::list_topics(region, PAGE, None)?,
                self.lookup.profile(),
            )
            .await
            .map(|a| json!({"count": a["Topics"].as_array().map_or(0, Vec::len), "more": next_token(&a).is_some()}))
            .unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
            per_region.push(json!({"region": region, "queues": queues, "topics": topics}));
        }
        Ok(json!({
            "identity": {"account": identity["Account"], "arn": identity["Arn"]},
            "profile": self.lookup.profile(),
            "regions": regions,
            "perRegion": per_region,
            "reads": ["queues", "queue", "peek", "redrive_status", "topics", "topic"],
            "writes": if self.config.writes() { json!(["send", "publish", "redrive", "redrive_cancel", "drain"]) } else { json!([]) },
            "writesHint": if self.config.writes() { "each write previews unless confirm=true; drain saves every message to disk before deleting it" } else { "register with MESSAGING_WRITES=true to publish send, publish, redrive, redrive_cancel and drain" },
            "snapshotDir": self.config.snapshot_dir().display().to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{FakeRunner, TOPIC, URL};
    use super::*;
    use aws_mcp::domain::ServerConfig;

    fn discover(runner: &Arc<FakeRunner>, regions: Option<&str>, writes: &str) -> DiscoverScope {
        let runner: Arc<dyn aws_mcp::ports::AwsRunner> = runner.clone();
        DiscoverScope::new(
            Arc::new(QueueLookup::new(runner, None)),
            MessagingConfig::new(
                ServerConfig::new(regions, None, None).unwrap(),
                Some(writes),
                Some("/snap"),
                None,
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn the_scope_lists_queue_names_and_topics_per_region() {
        let runner = FakeRunner::answering(vec![
            (
                "sts get-caller-identity",
                Ok(json!({"Account": "123456789012", "Arn": "arn:x"})),
            ),
            (
                "sqs list-queues",
                Ok(
                    json!({"QueueUrls": [URL, "https://sqs.eu-west-1.amazonaws.com/123456789012/demo-orders"], "NextToken": "t"}),
                ),
            ),
            (
                "sns list-topics",
                Ok(json!({"Topics": [{"TopicArn": TOPIC}]})),
            ),
        ]);
        let answer = discover(&runner, Some("eu-west-1,us-east-1"), "true")
            .execute(None)
            .await
            .unwrap();
        assert_eq!(answer["identity"]["account"], "123456789012");
        assert_eq!(answer["perRegion"].as_array().unwrap().len(), 2);
        assert_eq!(answer["perRegion"][0]["queues"]["count"], 2);
        assert_eq!(answer["perRegion"][0]["queues"]["deadLetterByName"], 1);
        assert_eq!(answer["perRegion"][0]["queues"]["more"], true);
        assert_eq!(answer["perRegion"][1]["topics"]["count"], 1);
        assert_eq!(answer["writes"].as_array().unwrap().len(), 5);
        assert_eq!(answer["snapshotDir"], "/snap");
        let one = discover(&runner, Some("eu-west-1,us-east-1"), "false")
            .execute(Some("us-east-1"))
            .await
            .unwrap();
        assert_eq!(one["regions"], json!(["us-east-1"]));
        assert_eq!(one["writes"], json!([]));
        assert!(
            discover(&runner, Some("eu-west-1"), "false")
                .execute(Some("ap-south-1"))
                .await
                .is_err()
        );
        assert!(
            discover(&runner, None, "false")
                .execute(None)
                .await
                .is_err()
        );
        runner.set(vec![
            ("sts get-caller-identity", Ok(json!({"Account": "123456789012"}))),
            ("sqs list-queues", Err("An error occurred (AccessDeniedException) when calling the ListQueues operation: no".into())),
            ("sns list-topics", Err("An error occurred (AccessDeniedException) when calling the ListTopics operation: no".into())),
        ]);
        let denied = discover(&runner, Some("eu-west-1"), "false")
            .execute(None)
            .await
            .unwrap();
        assert!(
            denied["perRegion"][0]["queues"]["error"]
                .as_str()
                .unwrap()
                .contains("ListQueues")
        );
        assert!(
            denied["perRegion"][0]["topics"]["error"]
                .as_str()
                .unwrap()
                .contains("ListTopics")
        );
        runner.set(vec![(
            "sts get-caller-identity",
            Err("Unable to locate credentials".into()),
        )]);
        assert!(
            discover(&runner, Some("eu-west-1"), "false")
                .execute(None)
                .await
                .is_err()
        );
    }
}
