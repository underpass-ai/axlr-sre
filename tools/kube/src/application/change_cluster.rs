use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{TEXT_MAX, bounded, failure, items_of, run_json, run_ok};
use crate::domain::{Invocation, Manifest, Policy, RolloutAction, RolloutTarget, ScaleTarget};
use crate::ports::{Completed, KubectlRunner};

const NEXT: &str = "call again with confirm=true to run it";

/// The writes: `diff`, `apply`, `rollout_restart`, `rollout_undo`, `scale`.
/// Every one previews unless `confirm` is true; the preview path never
/// reaches the write. The policy must allow writes, else nothing runs.
pub struct ChangeCluster {
    runner: Arc<dyn KubectlRunner>,
    policy: Policy,
}

impl ChangeCluster {
    pub fn new(runner: Arc<dyn KubectlRunner>, policy: Policy) -> Self {
        Self { runner, policy }
    }

    /// `kubectl diff`: exit 0 is no differences, 1 is differences, more is an error.
    pub async fn diff(&self, namespace: &str, manifest: &str) -> Result<Value> {
        self.policy.require_writes()?;
        let manifest = Manifest::parse(manifest, namespace)?;
        let mut answer = self.run_diff(namespace, &manifest).await?;
        answer["objects"] = json!(manifest.labels());
        Ok(answer)
    }

    async fn run_diff(&self, namespace: &str, manifest: &Manifest) -> Result<Value> {
        let invocation = Invocation::diff(namespace, manifest)?;
        let completed = self.runner.run(&invocation).await?;
        match completed.exit_code {
            Some(0) => Ok(json!({"differences": false, "diff": ""})),
            Some(1) => {
                let (diff, truncated) = bounded(&completed.stdout, TEXT_MAX);
                Ok(json!({"differences": true, "diff": diff, "truncated": truncated}))
            }
            _ => Err(failure(&invocation, &completed)),
        }
    }

    /// Without `confirm`: the diff and a server dry run. With it: the diff,
    /// the dry run and then the apply, so nothing is applied that the
    /// server would have refused.
    pub async fn apply(&self, namespace: &str, manifest: &str, confirm: bool) -> Result<Value> {
        self.policy.require_writes()?;
        let manifest = Manifest::parse(manifest, namespace)?;
        let diff = self.run_diff(namespace, &manifest).await?;
        let dry_run = run_ok(
            self.runner.as_ref(),
            &Invocation::apply_dry_run(namespace, &manifest)?,
        )
        .await?;
        let mut answer = json!({
            "namespace": namespace,
            "objects": manifest.labels(),
            "differences": diff["differences"],
            "diff": diff["diff"],
            "dryRun": names(&dry_run),
        });
        if !confirm {
            answer["mode"] = json!("preview");
            answer["next"] = json!(NEXT);
            return Ok(answer);
        }
        let applied = run_ok(
            self.runner.as_ref(),
            &Invocation::apply(namespace, &manifest)?,
        )
        .await?;
        answer["mode"] = json!("applied");
        answer["applied"] = json!(names(&applied));
        Ok(answer)
    }

    /// `restart` or `undo`; the preview shows the target as it is now.
    pub async fn rollout(
        &self,
        namespace: &str,
        action: RolloutAction,
        target: RolloutTarget,
        name: &str,
        revision: Option<i64>,
        confirm: bool,
    ) -> Result<Value> {
        self.policy.require_writes()?;
        let invocation = Invocation::rollout(namespace, action, target, name, 1, revision)?;
        let current = self.current(namespace, target.as_str(), name).await?;
        if !confirm {
            return Ok(json!({
                "mode": "preview",
                "target": format!("{}/{name}", target.as_str()),
                "current": current,
                "command": invocation.display(),
                "next": NEXT,
            }));
        }
        let completed = run_ok(self.runner.as_ref(), &invocation).await?;
        Ok(json!({
            "mode": "done",
            "target": format!("{}/{name}", target.as_str()),
            "before": current,
            "detail": completed.stdout.trim(),
            "next": "call rollout_status to wait for it",
        }))
    }

    /// Replicas inside the policy's cap; the preview shows the current count.
    pub async fn scale(
        &self,
        namespace: &str,
        target: ScaleTarget,
        name: &str,
        replicas: i64,
        confirm: bool,
    ) -> Result<Value> {
        self.policy.require_writes()?;
        let replicas = self.policy.replicas(replicas)?;
        let invocation = Invocation::scale(namespace, target, name, replicas)?;
        let current = self.current(namespace, target.as_str(), name).await?;
        let mut answer = json!({
            "target": format!("{}/{name}", target.as_str()),
            "current": current,
            "requested": replicas,
            "max": self.policy.max_replicas(),
        });
        if !confirm {
            answer["mode"] = json!("preview");
            answer["command"] = json!(invocation.display());
            answer["next"] = json!(NEXT);
            return Ok(answer);
        }
        let completed = run_ok(self.runner.as_ref(), &invocation).await?;
        answer["mode"] = json!("done");
        answer["detail"] = json!(completed.stdout.trim());
        Ok(answer)
    }

    async fn current(&self, namespace: &str, kind: &str, name: &str) -> Result<Value> {
        let invocation = Invocation::get(namespace, kind, Some(name), None)?;
        let object = items_of(run_json(self.runner.as_ref(), &invocation).await?)
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        Ok(json!({
            "replicas": object["spec"]["replicas"],
            "ready": object["status"]["readyReplicas"],
            "generation": object["metadata"]["generation"],
            "revision": object["metadata"]["annotations"]["deployment.kubernetes.io/revision"],
        }))
    }
}

fn names(completed: &Completed) -> Vec<String> {
    completed
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::inspect_cluster::tests::{FakeRunner, exit};
    use super::*;

    const MANIFEST: &str = "kind: Deployment\nmetadata: {name: web}\nspec: {replicas: 2}\n";
    const DEPLOY: &str = r#"{"kind":"Deployment","metadata":{"name":"web","generation":4,"annotations":{"deployment.kubernetes.io/revision":"3"}},"spec":{"replicas":2},"status":{"readyReplicas":2}}"#;

    fn writer(answers: Vec<(&'static str, Completed)>) -> (Arc<FakeRunner>, ChangeCluster) {
        let runner = FakeRunner::answering(answers);
        let policy = Policy::new(vec![], true, 5).unwrap();
        (runner.clone(), ChangeCluster::new(runner, policy))
    }

    #[tokio::test]
    async fn diff_tells_differences_apart_from_errors() {
        let (_, change) = writer(vec![("diff", exit(1, "-replicas: 1\n+replicas: 2\n", ""))]);
        let answer = change.diff("ns", MANIFEST).await.unwrap();
        assert_eq!(answer["differences"], true);
        assert!(answer["diff"].as_str().unwrap().contains("+replicas: 2"));
        assert_eq!(answer["objects"], json!(["Deployment/web"]));
        let (_, same) = writer(vec![("diff", exit(0, "", ""))]);
        assert_eq!(
            same.diff("ns", MANIFEST).await.unwrap()["differences"],
            false
        );
        let (_, broken) = writer(vec![("diff", exit(2, "", "error: unable to connect"))]);
        assert!(
            broken
                .diff("ns", MANIFEST)
                .await
                .unwrap_err()
                .to_string()
                .contains("unable to connect")
        );
        assert!(
            change
                .diff("ns", "kind: Secret\nmetadata: {name: s}")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn apply_previews_then_applies_only_on_confirm() {
        let (runner, change) = writer(vec![
            ("diff", exit(1, "+x", "")),
            (
                "apply -f - -n ns --dry-run=server",
                exit(0, "deployment.apps/web\n", ""),
            ),
            (
                "apply -f - -n ns -o name",
                exit(0, "deployment.apps/web\n", ""),
            ),
        ]);
        let preview = change.apply("ns", MANIFEST, false).await.unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["dryRun"], json!(["deployment.apps/web"]));
        assert_eq!(preview["differences"], true);
        assert_eq!(runner.calls().len(), 2);
        assert!(!runner.calls().iter().any(|args| {
            args.join(" ").ends_with("-o name --request-timeout=30s")
                && !args.contains(&"--dry-run=server".to_string())
        }));
        let applied = change.apply("ns", MANIFEST, true).await.unwrap();
        assert_eq!(applied["mode"], "applied");
        assert_eq!(applied["applied"], json!(["deployment.apps/web"]));
        assert_eq!(runner.calls().len(), 5);
    }

    #[tokio::test]
    async fn a_refused_dry_run_stops_the_apply() {
        let (runner, change) = writer(vec![
            ("diff", exit(0, "", "")),
            (
                "apply -f - -n ns --dry-run=server",
                exit(1, "", "admission webhook denied"),
            ),
        ]);
        let error = change.apply("ns", MANIFEST, true).await.unwrap_err();
        assert!(error.to_string().contains("admission webhook denied"));
        assert_eq!(runner.calls().len(), 2);
    }

    #[tokio::test]
    async fn rollout_previews_the_current_state_and_runs_on_confirm() {
        let (runner, change) = writer(vec![
            ("get deployment web", exit(0, DEPLOY, "")),
            (
                "rollout restart",
                exit(0, "deployment.apps/web restarted\n", ""),
            ),
            (
                "rollout undo",
                exit(0, "deployment.apps/web rolled back\n", ""),
            ),
        ]);
        let preview = change
            .rollout(
                "ns",
                RolloutAction::Restart,
                RolloutTarget::Deployment,
                "web",
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["current"]["revision"], "3");
        assert_eq!(preview["current"]["generation"], 4);
        assert!(
            preview["command"]
                .as_str()
                .unwrap()
                .starts_with("kubectl rollout restart deployment/web")
        );
        assert_eq!(runner.calls().len(), 1);
        let done = change
            .rollout(
                "ns",
                RolloutAction::Undo,
                RolloutTarget::Deployment,
                "web",
                Some(2),
                true,
            )
            .await
            .unwrap();
        assert_eq!(done["mode"], "done");
        assert!(done["detail"].as_str().unwrap().contains("rolled back"));
        assert!(runner.calls()[2].contains(&"--to-revision".to_string()));
        assert!(
            change
                .rollout(
                    "ns",
                    RolloutAction::Undo,
                    RolloutTarget::Deployment,
                    "web",
                    Some(0),
                    true
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn scale_stays_inside_the_cap() {
        let (runner, change) = writer(vec![
            ("get deployment web", exit(0, DEPLOY, "")),
            ("scale", exit(0, "deployment.apps/web scaled\n", "")),
        ]);
        let preview = change
            .scale("ns", ScaleTarget::Deployment, "web", 3, false)
            .await
            .unwrap();
        assert_eq!(preview["mode"], "preview");
        assert_eq!(preview["current"]["replicas"], 2);
        assert_eq!(preview["requested"], 3);
        assert_eq!(preview["max"], 5);
        assert_eq!(runner.calls().len(), 1);
        let done = change
            .scale("ns", ScaleTarget::Deployment, "web", 0, true)
            .await
            .unwrap();
        assert_eq!(done["mode"], "done");
        assert!(runner.calls()[2].contains(&"--replicas".to_string()));
        assert!(
            change
                .scale("ns", ScaleTarget::Deployment, "web", 6, true)
                .await
                .is_err()
        );
        assert_eq!(runner.calls().len(), 3);
        let (_, missing) = writer(vec![(
            "get deployment web",
            exit(0, r#"{"kind":"List","items":[]}"#, ""),
        )]);
        let preview = missing
            .scale("ns", ScaleTarget::Deployment, "web", 1, false)
            .await
            .unwrap();
        assert!(preview["current"]["replicas"].is_null());
    }

    #[tokio::test]
    async fn a_read_only_policy_runs_nothing() {
        let runner = FakeRunner::answering(vec![]);
        let change = ChangeCluster::new(runner.clone(), Policy::read_only());
        assert!(change.diff("ns", MANIFEST).await.is_err());
        assert!(change.apply("ns", MANIFEST, true).await.is_err());
        assert!(
            change
                .rollout(
                    "ns",
                    RolloutAction::Restart,
                    RolloutTarget::Deployment,
                    "web",
                    None,
                    true
                )
                .await
                .is_err()
        );
        assert!(
            change
                .scale("ns", ScaleTarget::Deployment, "web", 1, true)
                .await
                .is_err()
        );
        assert!(runner.calls().is_empty());
    }
}
