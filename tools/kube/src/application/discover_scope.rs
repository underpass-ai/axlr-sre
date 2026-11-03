use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::shared::{items_of, run_json, summarize};
use crate::domain::{Invocation, Policy};
use crate::ports::KubectlRunner;

const NAMESPACES_MAX: usize = 10;
const NAMES_MAX: usize = 20;

/// `discover`: for each namespace in scope, the pods by phase and the ones
/// not ready, the deployments and the ones short of replicas, and how many
/// warnings the namespace holds. Bounded: ten namespaces, twenty names each.
pub struct DiscoverScope {
    runner: Arc<dyn KubectlRunner>,
    policy: Policy,
}

impl DiscoverScope {
    pub fn new(runner: Arc<dyn KubectlRunner>, policy: Policy) -> Self {
        Self { runner, policy }
    }

    pub async fn execute(&self, namespace: Option<&str>) -> Result<Value> {
        let namespaces: Vec<String> = match namespace {
            Some(namespace) => vec![self.policy.namespace(Some(namespace))?],
            None if self.policy.namespaces().is_empty() => {
                return Err(anyhow!(
                    "name a namespace: this server has no allowlist to walk"
                ));
            }
            None => self
                .policy
                .namespaces()
                .iter()
                .take(NAMESPACES_MAX)
                .cloned()
                .collect(),
        };
        let mut scopes = Vec::new();
        for namespace in &namespaces {
            scopes.push(self.scope_of(namespace).await?);
        }
        Ok(json!({
            "mode": if self.policy.writes() { "read-write" } else { "read-only" },
            "namespaces": scopes,
            "truncated": self.policy.namespaces().len() > namespaces.len() && namespace.is_none(),
        }))
    }

    async fn scope_of(&self, namespace: &str) -> Result<Value> {
        let pods = items_of(
            run_json(
                self.runner.as_ref(),
                &Invocation::get(namespace, "pods", None, None)?,
            )
            .await?,
        );
        let mut by_phase: BTreeMap<String, usize> = BTreeMap::new();
        let mut not_ready = Vec::new();
        for pod in &pods {
            let summary = summarize(pod);
            *by_phase
                .entry(summary["phase"].as_str().unwrap_or("Unknown").to_string())
                .or_default() += 1;
            if !summary["reason"].is_null() && not_ready.len() < NAMES_MAX {
                not_ready.push(json!({"name": summary["name"], "reason": summary["reason"], "restarts": summary["restarts"]}));
            }
        }
        let deployments = items_of(
            run_json(
                self.runner.as_ref(),
                &Invocation::get(namespace, "deployments", None, None)?,
            )
            .await?,
        );
        let short: Vec<Value> = deployments
            .iter()
            .map(summarize)
            .filter(|d| d["ready"].as_u64().unwrap_or(0) < d["replicas"].as_u64().unwrap_or(0))
            .take(NAMES_MAX)
            .map(|d| json!({"name": d["name"], "ready": d["ready"], "replicas": d["replicas"]}))
            .collect();
        let warnings = items_of(
            run_json(
                self.runner.as_ref(),
                &Invocation::events(namespace, None, true)?,
            )
            .await?,
        );
        Ok(json!({
            "namespace": namespace,
            "pods": {"total": pods.len(), "byPhase": by_phase, "notReady": not_ready},
            "deployments": {"total": deployments.len(), "short": short},
            "warnings": warnings.len(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::inspect_cluster::tests::{FakeRunner, exit};
    use super::*;

    const PODS: &str = r#"{"kind":"List","items":[
        {"kind":"Pod","metadata":{"name":"ok"},"status":{"phase":"Running","containerStatuses":[{"ready":true}]}},
        {"kind":"Pod","metadata":{"name":"bad"},"status":{"phase":"Running","containerStatuses":[{"ready":false,"restartCount":7,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}},
        {"kind":"Pod","metadata":{"name":"new"},"status":{"phase":"Pending"}}]}"#;
    const DEPLOYMENTS: &str = r#"{"kind":"List","items":[
        {"kind":"Deployment","metadata":{"name":"web"},"spec":{"replicas":3},"status":{"readyReplicas":1}},
        {"kind":"Deployment","metadata":{"name":"api"},"spec":{"replicas":2},"status":{"readyReplicas":2}}]}"#;
    const WARNINGS: &str = r#"{"kind":"List","items":[{"type":"Warning"},{"type":"Warning"}]}"#;

    fn runner() -> Arc<FakeRunner> {
        FakeRunner::answering(vec![
            ("get pods", exit(0, PODS, "")),
            ("get deployments", exit(0, DEPLOYMENTS, "")),
            ("get events", exit(0, WARNINGS, "")),
        ])
    }

    #[tokio::test]
    async fn the_allowlist_is_walked_and_each_namespace_summarised() {
        let policy = Policy::new(vec!["prod".into(), "stage".into()], false, 1).unwrap();
        let scope = DiscoverScope::new(runner(), policy)
            .execute(None)
            .await
            .unwrap();
        assert_eq!(scope["mode"], "read-only");
        assert_eq!(scope["truncated"], false);
        assert_eq!(scope["namespaces"].as_array().unwrap().len(), 2);
        let prod = &scope["namespaces"][0];
        assert_eq!(prod["namespace"], "prod");
        assert_eq!(prod["pods"]["total"], 3);
        assert_eq!(prod["pods"]["byPhase"]["Running"], 2);
        assert_eq!(prod["pods"]["notReady"][0]["reason"], "CrashLoopBackOff");
        assert_eq!(prod["deployments"]["short"][0]["name"], "web");
        assert_eq!(prod["deployments"]["short"].as_array().unwrap().len(), 1);
        assert_eq!(prod["warnings"], 2);
    }

    #[tokio::test]
    async fn a_named_namespace_is_the_only_one_and_must_be_allowed() {
        let policy = Policy::new(vec!["prod".into()], true, 1).unwrap();
        let discover = DiscoverScope::new(runner(), policy);
        let scope = discover.execute(Some("prod")).await.unwrap();
        assert_eq!(scope["mode"], "read-write");
        assert_eq!(scope["namespaces"].as_array().unwrap().len(), 1);
        assert!(discover.execute(Some("other")).await.is_err());
    }

    #[tokio::test]
    async fn without_an_allowlist_a_namespace_must_be_named() {
        let discover = DiscoverScope::new(runner(), Policy::read_only());
        assert!(
            discover
                .execute(None)
                .await
                .unwrap_err()
                .to_string()
                .contains("name a namespace")
        );
        assert_eq!(
            discover.execute(Some("any")).await.unwrap()["namespaces"][0]["namespace"],
            "any"
        );
    }

    #[tokio::test]
    async fn a_failing_read_fails_the_discovery() {
        let runner = FakeRunner::answering(vec![("get pods", exit(1, "", "forbidden"))]);
        let discover = DiscoverScope::new(runner, Policy::read_only());
        assert!(
            discover
                .execute(Some("x"))
                .await
                .unwrap_err()
                .to_string()
                .contains("forbidden")
        );
    }
}
