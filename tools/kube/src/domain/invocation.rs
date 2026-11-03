use super::{InputError, Manifest, tokens};

pub const LOG_TAIL_MAX: i64 = 1000;
pub const LOG_BYTES_MAX: u64 = 262_144;
pub const ROLLOUT_TIMEOUT_MAX: i64 = 600;
const REQUEST_TIMEOUT: &str = "--request-timeout=30s";

/// What `rollout` may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutTarget {
    Deployment,
    StatefulSet,
    DaemonSet,
}

impl RolloutTarget {
    pub fn parse(value: &str) -> Result<Self, InputError> {
        match value.trim() {
            "deployment" => Ok(Self::Deployment),
            "statefulset" => Ok(Self::StatefulSet),
            "daemonset" => Ok(Self::DaemonSet),
            other => Err(InputError::UnknownChoice {
                what: "rollout kind",
                value: other.to_string(),
                allowed: "deployment, statefulset, daemonset",
            }),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::StatefulSet => "statefulset",
            Self::DaemonSet => "daemonset",
        }
    }
}

/// What `scale` may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleTarget {
    Deployment,
    StatefulSet,
    ReplicaSet,
}

impl ScaleTarget {
    pub fn parse(value: &str) -> Result<Self, InputError> {
        match value.trim() {
            "deployment" => Ok(Self::Deployment),
            "statefulset" => Ok(Self::StatefulSet),
            "replicaset" => Ok(Self::ReplicaSet),
            other => Err(InputError::UnknownChoice {
                what: "scale kind",
                value: other.to_string(),
                allowed: "deployment, statefulset, replicaset",
            }),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::StatefulSet => "statefulset",
            Self::ReplicaSet => "replicaset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutAction {
    Status,
    History,
    Restart,
    Undo,
}

impl RolloutAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::History => "history",
            Self::Restart => "restart",
            Self::Undo => "undo",
        }
    }
}

/// One `kubectl` run: its arguments, every one validated, and what goes to
/// its stdin. The runner adds the context and the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    args: Vec<String>,
    stdin: Option<String>,
    wait_seconds: Option<u64>,
}

fn strings<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.into_iter().map(str::to_string).collect()
}

impl Invocation {
    fn new(args: Vec<String>) -> Self {
        Self {
            args,
            stdin: None,
            wait_seconds: None,
        }
    }

    fn with_stdin(mut self, stdin: &str) -> Self {
        self.stdin = Some(stdin.to_string());
        self
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn stdin(&self) -> Option<&str> {
        self.stdin.as_deref()
    }

    /// How long this run legitimately waits (a rollout status), so the
    /// runner's kill timer stretches past it.
    #[must_use]
    pub fn wait_seconds(&self) -> Option<u64> {
        self.wait_seconds
    }

    /// The verb, for error messages.
    #[must_use]
    pub fn verb(&self) -> &str {
        self.args.first().map_or("kubectl", String::as_str)
    }

    /// The command as a person would type it, stdin aside.
    #[must_use]
    pub fn display(&self) -> String {
        format!("kubectl {}", self.args.join(" "))
    }

    pub fn get(
        namespace: &str,
        kind: &str,
        name: Option<&str>,
        selector: Option<&str>,
    ) -> Result<Self, InputError> {
        let mut args = strings(["get", &tokens::kind(kind)?]);
        if let Some(name) = name {
            args.push(tokens::name(name)?);
        }
        args.extend(strings([
            "-n",
            &tokens::namespace(namespace)?,
            "-o",
            "json",
            REQUEST_TIMEOUT,
        ]));
        if let Some(selector) = selector {
            args.extend(strings(["-l", &tokens::selector(selector)?]));
        }
        Ok(Self::new(args))
    }

    pub fn describe(namespace: &str, kind: &str, name: &str) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "describe",
            &tokens::kind(kind)?,
            &tokens::name(name)?,
            "-n",
            &tokens::namespace(namespace)?,
            REQUEST_TIMEOUT,
        ])))
    }

    pub fn logs(
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        tail: i64,
        since: Option<&str>,
        previous: bool,
    ) -> Result<Self, InputError> {
        if !(1..=LOG_TAIL_MAX).contains(&tail) {
            return Err(InputError::OutOfRange {
                what: "tail",
                min: 1,
                max: LOG_TAIL_MAX,
            });
        }
        let mut args = strings([
            "logs",
            &tokens::name(pod)?,
            "-n",
            &tokens::namespace(namespace)?,
            "--tail",
            &tail.to_string(),
            "--limit-bytes",
            &LOG_BYTES_MAX.to_string(),
            REQUEST_TIMEOUT,
        ]);
        if let Some(container) = container {
            args.extend(strings(["-c", &tokens::container(container)?]));
        }
        if let Some(since) = since {
            args.extend(strings(["--since", &tokens::duration(since)?]));
        }
        if previous {
            args.push("--previous".to_string());
        }
        Ok(Self::new(args))
    }

    pub fn events(
        namespace: &str,
        object: Option<&str>,
        warnings_only: bool,
    ) -> Result<Self, InputError> {
        let mut args = strings([
            "get",
            "events",
            "-n",
            &tokens::namespace(namespace)?,
            "-o",
            "json",
            REQUEST_TIMEOUT,
        ]);
        let mut fields = Vec::new();
        if let Some(object) = object {
            fields.push(format!("involvedObject.name={}", tokens::name(object)?));
        }
        if warnings_only {
            fields.push("type=Warning".to_string());
        }
        if !fields.is_empty() {
            args.extend(strings(["--field-selector", &fields.join(",")]));
        }
        Ok(Self::new(args))
    }

    pub fn diff(namespace: &str, manifest: &Manifest) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "diff",
            "-f",
            "-",
            "-n",
            &tokens::namespace(namespace)?,
            REQUEST_TIMEOUT,
        ]))
        .with_stdin(manifest.text()))
    }

    pub fn apply_dry_run(namespace: &str, manifest: &Manifest) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "apply",
            "-f",
            "-",
            "-n",
            &tokens::namespace(namespace)?,
            "--dry-run=server",
            "-o",
            "name",
            REQUEST_TIMEOUT,
        ]))
        .with_stdin(manifest.text()))
    }

    pub fn apply(namespace: &str, manifest: &Manifest) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "apply",
            "-f",
            "-",
            "-n",
            &tokens::namespace(namespace)?,
            "-o",
            "name",
            REQUEST_TIMEOUT,
        ]))
        .with_stdin(manifest.text()))
    }

    /// `status` waits up to `timeout_seconds`; `undo` may name a revision.
    pub fn rollout(
        namespace: &str,
        action: RolloutAction,
        target: RolloutTarget,
        name: &str,
        timeout_seconds: i64,
        revision: Option<i64>,
    ) -> Result<Self, InputError> {
        let mut args = strings([
            "rollout",
            action.as_str(),
            &format!("{}/{}", target.as_str(), tokens::name(name)?),
            "-n",
            &tokens::namespace(namespace)?,
            REQUEST_TIMEOUT,
        ]);
        if action == RolloutAction::Status {
            if !(1..=ROLLOUT_TIMEOUT_MAX).contains(&timeout_seconds) {
                return Err(InputError::OutOfRange {
                    what: "timeout_seconds",
                    min: 1,
                    max: ROLLOUT_TIMEOUT_MAX,
                });
            }
            args.extend(strings(["--timeout", &format!("{timeout_seconds}s")]));
            return Ok(Self {
                wait_seconds: Some(timeout_seconds.unsigned_abs()),
                ..Self::new(args)
            });
        }
        if action == RolloutAction::Undo
            && let Some(revision) = revision
        {
            if revision < 1 {
                return Err(InputError::OutOfRange {
                    what: "revision",
                    min: 1,
                    max: i64::MAX,
                });
            }
            args.extend(strings(["--to-revision", &revision.to_string()]));
        }
        Ok(Self::new(args))
    }

    /// Replicas already bounded by the policy.
    pub fn scale(
        namespace: &str,
        target: ScaleTarget,
        name: &str,
        replicas: u32,
    ) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "scale",
            &format!("{}/{}", target.as_str(), tokens::name(name)?),
            "-n",
            &tokens::namespace(namespace)?,
            "--replicas",
            &replicas.to_string(),
            REQUEST_TIMEOUT,
        ])))
    }

    #[must_use]
    pub fn client_version() -> Self {
        Self::new(strings(["version", "--client", "-o", "json"]))
    }

    #[must_use]
    pub fn version() -> Self {
        Self::new(strings(["version", "-o", "json", REQUEST_TIMEOUT]))
    }

    /// `kubectl auth can-i <verb> <resource> -n <namespace>`: exit 0 yes, 1 no.
    pub fn can_i(namespace: &str, verb: &str, resource: &str) -> Result<Self, InputError> {
        Ok(Self::new(strings([
            "auth",
            "can-i",
            &tokens::kind(verb)?,
            &tokens::kind(resource)?,
            "-n",
            &tokens::namespace(namespace)?,
            REQUEST_TIMEOUT,
        ])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse("kind: ConfigMap\nmetadata: {name: c}\n", "prod").unwrap()
    }

    #[test]
    fn reads_build_their_arguments_in_order() {
        let get = Invocation::get("prod", "pods", None, Some("app=web")).unwrap();
        assert_eq!(
            get.args(),
            [
                "get",
                "pods",
                "-n",
                "prod",
                "-o",
                "json",
                REQUEST_TIMEOUT,
                "-l",
                "app=web"
            ]
        );
        assert_eq!(get.verb(), "get");
        assert_eq!(
            get.display(),
            format!("kubectl get pods -n prod -o json {REQUEST_TIMEOUT} -l app=web")
        );
        assert!(get.stdin().is_none());
        let one = Invocation::get("prod", "deploy", Some("web"), None).unwrap();
        assert_eq!(one.args()[2], "web");
        let describe = Invocation::describe("prod", "pod", "web-1").unwrap();
        assert_eq!(describe.args()[..3], ["describe", "pod", "web-1"]);
        let logs = Invocation::logs("prod", "web-1", Some("app"), 50, Some("5m"), true).unwrap();
        assert_eq!(
            logs.args()[4..8],
            ["--tail", "50", "--limit-bytes", "262144"]
        );
        assert!(logs.args().ends_with(&[
            "-c".into(),
            "app".into(),
            "--since".into(),
            "5m".into(),
            "--previous".into()
        ]));
        let events = Invocation::events("prod", Some("web-1"), true).unwrap();
        assert_eq!(
            events.args()[7..],
            ["--field-selector", "involvedObject.name=web-1,type=Warning"]
        );
        assert_eq!(
            Invocation::events("prod", None, false)
                .unwrap()
                .args()
                .len(),
            7
        );
    }

    #[test]
    fn writes_send_the_manifest_on_stdin() {
        let diff = Invocation::diff("prod", &manifest()).unwrap();
        assert_eq!(diff.args()[..3], ["diff", "-f", "-"]);
        assert_eq!(diff.stdin().unwrap(), manifest().text());
        assert!(
            Invocation::apply_dry_run("prod", &manifest())
                .unwrap()
                .args()
                .contains(&"--dry-run=server".to_string())
        );
        let apply = Invocation::apply("prod", &manifest()).unwrap();
        assert!(!apply.args().iter().any(|a| a.contains("dry-run")));
        assert_eq!(apply.stdin().unwrap(), manifest().text());
    }

    #[test]
    fn rollout_and_scale_name_their_target() {
        let status = Invocation::rollout(
            "prod",
            RolloutAction::Status,
            RolloutTarget::Deployment,
            "web",
            120,
            None,
        )
        .unwrap();
        assert_eq!(status.args()[..3], ["rollout", "status", "deployment/web"]);
        assert!(
            status
                .args()
                .ends_with(&["--timeout".into(), "120s".into()])
        );
        let undo = Invocation::rollout(
            "prod",
            RolloutAction::Undo,
            RolloutTarget::StatefulSet,
            "db",
            1,
            Some(3),
        )
        .unwrap();
        assert!(undo.args().ends_with(&["--to-revision".into(), "3".into()]));
        let restart = Invocation::rollout(
            "prod",
            RolloutAction::Restart,
            RolloutTarget::DaemonSet,
            "agent",
            1,
            Some(3),
        )
        .unwrap();
        assert!(
            !restart
                .args()
                .iter()
                .any(|a| a.contains("revision") || a.contains("timeout") && a != REQUEST_TIMEOUT)
        );
        let history = Invocation::rollout(
            "prod",
            RolloutAction::History,
            RolloutTarget::Deployment,
            "web",
            1,
            None,
        )
        .unwrap();
        assert_eq!(history.args()[1], "history");
        let scale = Invocation::scale("prod", ScaleTarget::ReplicaSet, "web-1", 3).unwrap();
        assert_eq!(scale.args()[1], "replicaset/web-1");
        assert!(
            scale
                .args()
                .ends_with(&["--replicas".into(), "3".into(), REQUEST_TIMEOUT.into()])
        );
    }

    #[test]
    fn bounds_are_checked_before_kubectl() {
        assert!(Invocation::logs("prod", "p", None, 0, None, false).is_err());
        assert!(Invocation::logs("prod", "p", None, 1001, None, false).is_err());
        assert!(Invocation::logs("prod", "p", None, 10, Some("1d"), false).is_err());
        assert!(Invocation::logs("prod", "p", Some("Bad"), 10, None, false).is_err());
        assert!(
            Invocation::rollout(
                "prod",
                RolloutAction::Status,
                RolloutTarget::Deployment,
                "web",
                0,
                None
            )
            .is_err()
        );
        assert!(
            Invocation::rollout(
                "prod",
                RolloutAction::Status,
                RolloutTarget::Deployment,
                "web",
                601,
                None
            )
            .is_err()
        );
        assert!(
            Invocation::rollout(
                "prod",
                RolloutAction::Undo,
                RolloutTarget::Deployment,
                "web",
                1,
                Some(0)
            )
            .is_err()
        );
        assert!(Invocation::get("Bad", "pods", None, None).is_err());
        assert!(Invocation::get("prod", "secrets", None, None).is_err());
        assert!(Invocation::get("prod", "pods", Some("-o"), None).is_err());
        assert!(Invocation::get("prod", "pods", None, Some("-l")).is_err());
        assert!(Invocation::describe("prod", "pods", "../x").is_err());
        assert!(Invocation::events("prod", Some("Bad"), true).is_err());
        assert!(Invocation::scale("prod", ScaleTarget::Deployment, "Bad", 1).is_err());
        assert!(Invocation::diff("Bad", &manifest()).is_err());
        assert!(Invocation::apply("Bad", &manifest()).is_err());
        assert!(Invocation::apply_dry_run("Bad", &manifest()).is_err());
        assert!(Invocation::can_i("prod", "list", "pods").is_ok());
        assert!(Invocation::can_i("prod", "list", "secrets").is_err());
    }

    #[test]
    fn targets_parse_their_names_and_nothing_else() {
        assert_eq!(
            RolloutTarget::parse(" statefulset ").unwrap(),
            RolloutTarget::StatefulSet
        );
        assert_eq!(
            RolloutTarget::parse("daemonset").unwrap().as_str(),
            "daemonset"
        );
        assert!(matches!(
            RolloutTarget::parse("replicaset"),
            Err(InputError::UnknownChoice {
                what: "rollout kind",
                ..
            })
        ));
        assert_eq!(
            ScaleTarget::parse("replicaset").unwrap(),
            ScaleTarget::ReplicaSet
        );
        assert_eq!(
            ScaleTarget::parse("statefulset").unwrap().as_str(),
            "statefulset"
        );
        assert!(matches!(
            ScaleTarget::parse("daemonset"),
            Err(InputError::UnknownChoice {
                what: "scale kind",
                ..
            })
        ));
        assert_eq!(RolloutAction::History.as_str(), "history");
        assert_eq!(RolloutAction::Restart.as_str(), "restart");
    }

    #[test]
    fn versions_need_no_namespace() {
        assert_eq!(
            Invocation::client_version().args()[..2],
            ["version", "--client"]
        );
        assert_eq!(Invocation::version().args()[0], "version");
        assert_eq!(Invocation::version().verb(), "version");
    }
}
