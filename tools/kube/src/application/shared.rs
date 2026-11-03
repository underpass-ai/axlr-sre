//! What every use case does with a `kubectl` run: bound its text, turn a
//! non-zero exit into an error that names the verb, read its JSON.

use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};

use crate::domain::Invocation;
use crate::ports::{Completed, KubectlRunner};

/// The most text one answer carries, 64 KiB.
pub const TEXT_MAX: usize = 65_536;
const STDERR_MAX: usize = 2_048;

/// The first `max` bytes of a text, cut on a character boundary.
pub fn bounded(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// A text answer: `{"text", "truncated"}`.
pub fn text_answer(text: &str) -> Value {
    let (text, truncated) = bounded(text.trim_end(), TEXT_MAX);
    json!({"text": text, "truncated": truncated})
}

/// Runs and fails on any exit but zero, with the verb and what kubectl said.
pub async fn run_ok(runner: &dyn KubectlRunner, invocation: &Invocation) -> Result<Completed> {
    let completed = runner
        .run(invocation)
        .await
        .with_context(|| format!("kubectl {} could not run", invocation.verb()))?;
    if completed.succeeded() {
        Ok(completed)
    } else {
        Err(failure(invocation, &completed))
    }
}

pub fn failure(invocation: &Invocation, completed: &Completed) -> anyhow::Error {
    let (stderr, _) = bounded(completed.stderr.trim(), STDERR_MAX);
    let said = if stderr.is_empty() {
        bounded(completed.stdout.trim(), STDERR_MAX).0
    } else {
        stderr
    };
    match completed.exit_code {
        Some(code) => anyhow!("kubectl {} failed (exit {code}): {said}", invocation.verb()),
        None => anyhow!("kubectl {} was killed: {said}", invocation.verb()),
    }
}

/// Runs a `-o json` read and parses it.
pub async fn run_json(runner: &dyn KubectlRunner, invocation: &Invocation) -> Result<Value> {
    let completed = run_ok(runner, invocation).await?;
    serde_json::from_str(&completed.stdout)
        .with_context(|| format!("kubectl {} did not answer JSON", invocation.verb()))
}

/// The items of a `get`: a `List` gives its items, a single object itself.
pub fn items_of(value: Value) -> Vec<Value> {
    if value["kind"] == "List" {
        match value {
            Value::Object(mut object) => match object.remove("items") {
                Some(Value::Array(items)) => items,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    } else {
        vec![value]
    }
}

/// The object without `managedFields`, which nobody reads and doubles the size.
pub fn without_managed_fields(mut item: Value) -> Value {
    if let Some(Value::Object(metadata)) = item.get_mut("metadata") {
        metadata.remove("managedFields");
    }
    item
}

/// Name, namespace, kind, creation and a status line that depends on the kind.
pub fn summarize(item: &Value) -> Value {
    let metadata = &item["metadata"];
    let status = &item["status"];
    let spec = &item["spec"];
    let mut summary = Map::new();
    summary.insert("kind".into(), item["kind"].clone());
    summary.insert("name".into(), metadata["name"].clone());
    summary.insert("namespace".into(), metadata["namespace"].clone());
    summary.insert("created".into(), metadata["creationTimestamp"].clone());
    match item["kind"].as_str() {
        Some("Pod") => {
            let containers = status["containerStatuses"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let ready = containers.iter().filter(|c| c["ready"] == true).count();
            let restarts: u64 = containers
                .iter()
                .filter_map(|c| c["restartCount"].as_u64())
                .sum();
            let waiting = containers
                .iter()
                .find_map(|c| c["state"]["waiting"]["reason"].as_str().map(str::to_string))
                .or_else(|| {
                    containers.iter().find_map(|c| {
                        c["state"]["terminated"]["reason"]
                            .as_str()
                            .map(str::to_string)
                    })
                });
            summary.insert("phase".into(), status["phase"].clone());
            summary.insert(
                "ready".into(),
                json!(format!("{ready}/{}", containers.len())),
            );
            summary.insert("restarts".into(), json!(restarts));
            summary.insert("reason".into(), json!(waiting));
            summary.insert("node".into(), spec["nodeName"].clone());
        }
        Some("Deployment" | "StatefulSet" | "ReplicaSet") => {
            summary.insert("replicas".into(), spec["replicas"].clone());
            summary.insert(
                "ready".into(),
                json!(status["readyReplicas"].as_u64().unwrap_or(0)),
            );
            summary.insert(
                "available".into(),
                json!(status["availableReplicas"].as_u64().unwrap_or(0)),
            );
            summary.insert(
                "updated".into(),
                json!(status["updatedReplicas"].as_u64().unwrap_or(0)),
            );
        }
        Some("DaemonSet") => {
            summary.insert("desired".into(), status["desiredNumberScheduled"].clone());
            summary.insert("ready".into(), status["numberReady"].clone());
        }
        Some("Service") => {
            summary.insert("type".into(), spec["type"].clone());
            summary.insert("clusterIP".into(), spec["clusterIP"].clone());
        }
        Some("Node") => {
            let ready = status["conditions"]
                .as_array()
                .and_then(|conditions| conditions.iter().find(|c| c["type"] == "Ready"))
                .map(|c| c["status"].clone())
                .unwrap_or(Value::Null);
            summary.insert("ready".into(), ready);
        }
        Some("HorizontalPodAutoscaler") => {
            summary.insert("min".into(), spec["minReplicas"].clone());
            summary.insert("max".into(), spec["maxReplicas"].clone());
            summary.insert("current".into(), status["currentReplicas"].clone());
            summary.insert("desired".into(), status["desiredReplicas"].clone());
        }
        _ => {}
    }
    Value::Object(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_on_a_character_boundary() {
        let (text, truncated) = bounded("héllo", 2);
        assert_eq!(text, "h");
        assert!(truncated);
        assert_eq!(bounded("abc", 3), ("abc".into(), false));
        let answer = text_answer("x\n");
        assert_eq!(answer["text"], "x");
        assert_eq!(answer["truncated"], false);
    }

    #[test]
    fn a_failure_names_the_verb_and_prefers_stderr() {
        let invocation = Invocation::describe("ns", "pods", "x").unwrap();
        let killed = failure(
            &invocation,
            &Completed {
                exit_code: None,
                stdout: "out".into(),
                stderr: String::new(),
            },
        );
        assert_eq!(killed.to_string(), "kubectl describe was killed: out");
        let exited = failure(
            &invocation,
            &Completed {
                exit_code: Some(2),
                stdout: "out".into(),
                stderr: "bad\n".into(),
            },
        );
        assert_eq!(exited.to_string(), "kubectl describe failed (exit 2): bad");
    }

    #[test]
    fn items_come_from_a_list_or_a_single_object() {
        assert_eq!(
            items_of(json!({"kind": "List", "items": [{"a": 1}]})),
            vec![json!({"a": 1})]
        );
        assert_eq!(items_of(json!({"kind": "List"})), Vec::<Value>::new());
        assert_eq!(
            items_of(json!({"kind": "Pod"})),
            vec![json!({"kind": "Pod"})]
        );
        assert_eq!(items_of(json!("List")), vec![json!("List")]);
    }

    #[test]
    fn managed_fields_are_dropped() {
        let item = without_managed_fields(json!({"metadata": {"name": "x", "managedFields": [1]}}));
        assert_eq!(item, json!({"metadata": {"name": "x"}}));
        assert_eq!(without_managed_fields(json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn summaries_depend_on_the_kind() {
        let pod = summarize(
            &json!({"kind": "Pod", "metadata": {"name": "p", "namespace": "n"}, "spec": {"nodeName": "w1"},
            "status": {"phase": "Running", "containerStatuses": [{"ready": true, "restartCount": 2}, {"ready": false, "restartCount": 3, "state": {"waiting": {"reason": "CrashLoopBackOff"}}}]}}),
        );
        assert_eq!(pod["ready"], "1/2");
        assert_eq!(pod["restarts"], 5);
        assert_eq!(pod["reason"], "CrashLoopBackOff");
        assert_eq!(pod["node"], "w1");
        let terminated = summarize(
            &json!({"kind": "Pod", "status": {"containerStatuses": [{"state": {"terminated": {"reason": "OOMKilled"}}}]}}),
        );
        assert_eq!(terminated["reason"], "OOMKilled");
        let deploy = summarize(
            &json!({"kind": "Deployment", "spec": {"replicas": 3}, "status": {"readyReplicas": 1}}),
        );
        assert_eq!(deploy["replicas"], 3);
        assert_eq!(deploy["ready"], 1);
        assert_eq!(deploy["available"], 0);
        let daemon = summarize(
            &json!({"kind": "DaemonSet", "status": {"desiredNumberScheduled": 3, "numberReady": 3}}),
        );
        assert_eq!(daemon["desired"], 3);
        let service = summarize(
            &json!({"kind": "Service", "spec": {"type": "ClusterIP", "clusterIP": "10.0.0.1"}}),
        );
        assert_eq!(service["type"], "ClusterIP");
        let node = summarize(
            &json!({"kind": "Node", "status": {"conditions": [{"type": "Ready", "status": "False"}]}}),
        );
        assert_eq!(node["ready"], "False");
        let hpa = summarize(
            &json!({"kind": "HorizontalPodAutoscaler", "spec": {"minReplicas": 1, "maxReplicas": 5}, "status": {"desiredReplicas": 2}}),
        );
        assert_eq!(hpa["max"], 5);
        let other = summarize(&json!({"kind": "ConfigMap", "metadata": {"name": "c"}}));
        assert_eq!(other["name"], "c");
        assert!(other.get("phase").is_none());
    }
}
