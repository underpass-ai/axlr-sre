use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{items_of, run_json, run_ok, summarize, text_answer, without_managed_fields};
use crate::domain::{Invocation, RolloutAction, RolloutTarget};
use crate::ports::KubectlRunner;

pub const LIST_LIMIT_MAX: i64 = 200;

/// The reads: `get`, `describe`, `logs`, `events`, rollout status and
/// history. Every answer is bounded; nothing here changes the cluster.
pub struct InspectCluster {
    runner: Arc<dyn KubectlRunner>,
}

impl InspectCluster {
    pub fn new(runner: Arc<dyn KubectlRunner>) -> Self {
        Self { runner }
    }

    /// Summaries by default; `raw` gives the objects without `managedFields`.
    pub async fn get(
        &self,
        namespace: &str,
        kind: &str,
        name: Option<&str>,
        selector: Option<&str>,
        limit: i64,
        raw: bool,
    ) -> Result<Value> {
        let limit = bounded_limit(limit, "limit")?;
        let invocation = Invocation::get(namespace, kind, name, selector)?;
        let items = items_of(run_json(self.runner.as_ref(), &invocation).await?);
        let count = items.len();
        let shown: Vec<Value> = items
            .into_iter()
            .take(limit)
            .map(|item| {
                if raw {
                    without_managed_fields(item)
                } else {
                    summarize(&item)
                }
            })
            .collect();
        Ok(json!({
            "namespace": namespace,
            "count": count,
            "truncated": count > shown.len(),
            "items": shown,
        }))
    }

    pub async fn describe(&self, namespace: &str, kind: &str, name: &str) -> Result<Value> {
        let invocation = Invocation::describe(namespace, kind, name)?;
        let completed = run_ok(self.runner.as_ref(), &invocation).await?;
        Ok(text_answer(&completed.stdout))
    }

    pub async fn logs(
        &self,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        tail: i64,
        since: Option<&str>,
        previous: bool,
    ) -> Result<Value> {
        let invocation = Invocation::logs(namespace, pod, container, tail, since, previous)?;
        let completed = run_ok(self.runner.as_ref(), &invocation).await?;
        let lines: Vec<&str> = completed.stdout.lines().collect();
        Ok(json!({
            "pod": pod,
            "container": container,
            "previous": previous,
            "count": lines.len(),
            "lines": lines,
        }))
    }

    /// Newest first, `limit` at most.
    pub async fn events(
        &self,
        namespace: &str,
        object: Option<&str>,
        warnings_only: bool,
        limit: i64,
    ) -> Result<Value> {
        let limit = bounded_limit(limit, "limit")?;
        let invocation = Invocation::events(namespace, object, warnings_only)?;
        let mut items = items_of(run_json(self.runner.as_ref(), &invocation).await?);
        items.sort_by_cached_key(|event| std::cmp::Reverse(seen_at(event)));
        let count = items.len();
        let shown: Vec<Value> = items
            .iter()
            .take(limit)
            .map(|event| {
                json!({
                    "type": event["type"],
                    "reason": event["reason"],
                    "object": format!("{}/{}", event["involvedObject"]["kind"].as_str().unwrap_or("?"), event["involvedObject"]["name"].as_str().unwrap_or("?")),
                    "message": event["message"],
                    "count": event["count"],
                    "lastSeen": seen_at(event),
                })
            })
            .collect();
        Ok(json!({
            "namespace": namespace,
            "count": count,
            "truncated": count > shown.len(),
            "items": shown,
        }))
    }

    /// `complete`, `timed_out` (still rolling after `timeout_seconds`) or an error.
    pub async fn rollout_status(
        &self,
        namespace: &str,
        target: RolloutTarget,
        name: &str,
        timeout_seconds: i64,
    ) -> Result<Value> {
        let invocation = Invocation::rollout(
            namespace,
            RolloutAction::Status,
            target,
            name,
            timeout_seconds,
            None,
        )?;
        let completed = self.runner.run(&invocation).await?;
        if completed.succeeded() {
            return Ok(json!({"status": "complete", "detail": completed.stdout.trim()}));
        }
        if completed.stderr.contains("timed out") {
            return Ok(json!({
                "status": "timed_out",
                "detail": format!("still rolling after {timeout_seconds}s: {}", completed.stdout.trim()),
            }));
        }
        Err(super::shared::failure(&invocation, &completed))
    }

    pub async fn rollout_history(
        &self,
        namespace: &str,
        target: RolloutTarget,
        name: &str,
    ) -> Result<Value> {
        let invocation =
            Invocation::rollout(namespace, RolloutAction::History, target, name, 1, None)?;
        let completed = run_ok(self.runner.as_ref(), &invocation).await?;
        Ok(text_answer(&completed.stdout))
    }
}

fn bounded_limit(limit: i64, what: &'static str) -> Result<usize> {
    if (1..=LIST_LIMIT_MAX).contains(&limit) {
        Ok(usize::try_from(limit).expect("a positive limit fits"))
    } else {
        Err(crate::domain::InputError::OutOfRange {
            what,
            min: 1,
            max: LIST_LIMIT_MAX,
        }
        .into())
    }
}

fn seen_at(event: &Value) -> String {
    ["lastTimestamp", "eventTime"]
        .iter()
        .find_map(|key| event[key].as_str())
        .or_else(|| event["metadata"]["creationTimestamp"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;
    use async_trait::async_trait;

    use super::*;
    use crate::ports::Completed;

    /// Answers by verb; records every invocation.
    pub(crate) struct FakeRunner {
        pub answers: Vec<(&'static str, Completed)>,
        pub calls: Mutex<Vec<Vec<String>>>,
        pub fail_to_run: bool,
        pub diff_available: bool,
    }

    impl FakeRunner {
        pub fn answering(answers: Vec<(&'static str, Completed)>) -> Arc<Self> {
            Arc::new(Self {
                answers,
                calls: Mutex::new(Vec::new()),
                fail_to_run: false,
                diff_available: true,
            })
        }

        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    pub(crate) fn exit(code: i32, stdout: &str, stderr: &str) -> Completed {
        Completed {
            exit_code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[async_trait]
    impl KubectlRunner for FakeRunner {
        async fn run(&self, invocation: &Invocation) -> Result<Completed> {
            self.calls.lock().unwrap().push(invocation.args().to_vec());
            if self.fail_to_run {
                return Err(anyhow!("no such file"));
            }
            let args = invocation.args().join(" ");
            self.answers
                .iter()
                .find(|(prefix, _)| args.starts_with(prefix))
                .map(|(_, completed)| completed.clone())
                .ok_or_else(|| anyhow!("unexpected: {args}"))
        }

        fn program_available(&self, program: &str) -> bool {
            program != "diff" || self.diff_available
        }
    }

    const PODS: &str = r#"{"kind":"List","items":[
        {"kind":"Pod","metadata":{"name":"a","namespace":"ns","managedFields":[1]},"status":{"phase":"Running","containerStatuses":[{"ready":true,"restartCount":0}]}},
        {"kind":"Pod","metadata":{"name":"b","namespace":"ns"},"status":{"phase":"Pending"}}]}"#;

    #[tokio::test]
    async fn get_summarises_and_bounds() {
        let runner = FakeRunner::answering(vec![("get pods", exit(0, PODS, ""))]);
        let inspect = InspectCluster::new(runner.clone());
        let answer = inspect
            .get("ns", "pods", None, Some("app=x"), 1, false)
            .await
            .unwrap();
        assert_eq!(answer["count"], 2);
        assert_eq!(answer["truncated"], true);
        assert_eq!(answer["items"][0]["ready"], "1/1");
        assert!(runner.calls()[0].contains(&"app=x".to_string()));
        let raw = inspect
            .get("ns", "pods", None, None, 50, true)
            .await
            .unwrap();
        assert_eq!(raw["truncated"], false);
        assert!(raw["items"][0]["metadata"].get("managedFields").is_none());
        assert_eq!(raw["items"][1]["status"]["phase"], "Pending");
    }

    #[tokio::test]
    async fn get_fails_on_bad_limit_bad_input_bad_exit_and_bad_json() {
        let runner = FakeRunner::answering(vec![
            ("get pods", exit(0, "not json", "")),
            ("get deploy", exit(1, "", "Error from server (NotFound)")),
        ]);
        let inspect = InspectCluster::new(runner);
        assert!(
            inspect
                .get("ns", "pods", None, None, 0, false)
                .await
                .is_err()
        );
        assert!(
            inspect
                .get("ns", "pods", None, None, 201, false)
                .await
                .is_err()
        );
        assert!(
            inspect
                .get("ns", "secrets", None, None, 10, false)
                .await
                .is_err()
        );
        let json = inspect
            .get("ns", "pods", None, None, 10, false)
            .await
            .unwrap_err();
        assert!(json.to_string().contains("did not answer JSON"));
        let exit = inspect
            .get("ns", "deploy", Some("x"), None, 10, false)
            .await
            .unwrap_err();
        assert!(exit.to_string().contains("NotFound"), "{exit}");
        let unexpected = inspect
            .get("ns", "svc", None, None, 10, false)
            .await
            .unwrap_err();
        assert!(unexpected.to_string().contains("could not run"));
    }

    #[tokio::test]
    async fn describe_logs_and_history_are_text() {
        let runner = FakeRunner::answering(vec![
            ("describe", exit(0, "Name: a\nStatus: Running\n", "")),
            ("logs", exit(0, "l1\nl2\n", "")),
            ("rollout history", exit(0, "REVISION\n1\n2\n", "")),
        ]);
        let inspect = InspectCluster::new(runner.clone());
        let described = inspect.describe("ns", "pod", "a").await.unwrap();
        assert!(described["text"].as_str().unwrap().starts_with("Name: a"));
        let logs = inspect
            .logs("ns", "a", Some("app"), 10, None, true)
            .await
            .unwrap();
        assert_eq!(logs["count"], 2);
        assert_eq!(logs["lines"][1], "l2");
        assert_eq!(logs["previous"], true);
        let history = inspect
            .rollout_history("ns", RolloutTarget::Deployment, "web")
            .await
            .unwrap();
        assert!(history["text"].as_str().unwrap().contains("REVISION"));
        assert!(inspect.describe("ns", "pod", "Bad").await.is_err());
        assert!(inspect.logs("ns", "a", None, 0, None, false).await.is_err());
        assert!(
            inspect
                .rollout_history("Bad", RolloutTarget::Deployment, "web")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn events_come_newest_first() {
        let events = r#"{"kind":"List","items":[
            {"type":"Normal","reason":"Pulled","involvedObject":{"kind":"Pod","name":"a"},"message":"ok","count":1,"lastTimestamp":"2026-11-03T10:00:00Z"},
            {"type":"Warning","reason":"BackOff","involvedObject":{"kind":"Pod","name":"a"},"message":"crash","count":9,"eventTime":"2026-11-03T11:00:00Z","lastTimestamp":null},
            {"type":"Warning","reason":"Old","involvedObject":{},"message":"","metadata":{"creationTimestamp":"2026-11-01T00:00:00Z"}}]}"#;
        let runner = FakeRunner::answering(vec![("get events", exit(0, events, ""))]);
        let inspect = InspectCluster::new(runner.clone());
        let answer = inspect.events("ns", Some("a"), true, 2).await.unwrap();
        assert_eq!(answer["count"], 3);
        assert_eq!(answer["truncated"], true);
        assert_eq!(answer["items"][0]["reason"], "BackOff");
        assert_eq!(answer["items"][0]["object"], "Pod/a");
        assert_eq!(answer["items"][1]["reason"], "Pulled");
        assert!(runner.calls()[0].contains(&"involvedObject.name=a,type=Warning".to_string()));
        let all = inspect.events("ns", None, false, 10).await.unwrap();
        assert_eq!(all["items"][2]["object"], "?/?");
        assert!(inspect.events("ns", None, false, 0).await.is_err());
        assert!(inspect.events("ns", Some("Bad"), false, 10).await.is_err());
    }

    #[tokio::test]
    async fn rollout_status_tells_complete_from_timed_out_from_error() {
        let runner = FakeRunner::answering(vec![
            (
                "rollout status deployment/done",
                exit(0, "deployment \"done\" successfully rolled out\n", ""),
            ),
            (
                "rollout status deployment/slow",
                exit(
                    1,
                    "Waiting for 1 of 3\n",
                    "error: timed out waiting for the condition",
                ),
            ),
            (
                "rollout status deployment/gone",
                exit(1, "", "Error from server (NotFound)"),
            ),
        ]);
        let inspect = InspectCluster::new(runner);
        let done = inspect
            .rollout_status("ns", RolloutTarget::Deployment, "done", 30)
            .await
            .unwrap();
        assert_eq!(done["status"], "complete");
        let slow = inspect
            .rollout_status("ns", RolloutTarget::Deployment, "slow", 30)
            .await
            .unwrap();
        assert_eq!(slow["status"], "timed_out");
        assert!(slow["detail"].as_str().unwrap().contains("30s"));
        assert!(
            inspect
                .rollout_status("ns", RolloutTarget::Deployment, "gone", 30)
                .await
                .is_err()
        );
        assert!(
            inspect
                .rollout_status("ns", RolloutTarget::Deployment, "done", 0)
                .await
                .is_err()
        );
    }
}
