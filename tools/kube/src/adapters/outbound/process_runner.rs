use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::domain::Invocation;
use crate::ports::{Completed, KubectlRunner};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// Spawns `kubectl`. Its stdin, stdout and stderr are always pipes or null,
/// never inherited: this process's own stdio is the MCP protocol. The
/// external diff variable is dropped so no environment can swap the program
/// `kubectl diff` runs. A run that outlives the timeout is killed.
pub struct ProcessRunner {
    kubectl: PathBuf,
    context: Option<String>,
    timeout: Duration,
}

impl ProcessRunner {
    pub fn new(kubectl: impl AsRef<Path>, context: Option<String>) -> Self {
        Self {
            kubectl: kubectl.as_ref().to_path_buf(),
            context,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl KubectlRunner for ProcessRunner {
    async fn run(&self, invocation: &Invocation) -> Result<Completed> {
        let mut command = Command::new(&self.kubectl);
        if let Some(context) = &self.context {
            command.arg("--context").arg(context);
        }
        command
            .args(invocation.args())
            .env_remove("KUBECTL_EXTERNAL_DIFF")
            .stdin(if invocation.stdin().is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .with_context(|| format!("could not start {}", self.kubectl.display()))?;
        if let Some(text) = invocation.stdin()
            && let Some(mut stdin) = child.stdin.take()
        {
            let text = text.to_string();
            tokio::spawn(async move {
                let _ = stdin.write_all(text.as_bytes()).await;
                let _ = stdin.shutdown().await;
            });
        }
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                anyhow!(
                    "kubectl {} did not finish in {}s and was killed",
                    invocation.verb(),
                    self.timeout.as_secs()
                )
            })?
            .context("waiting for kubectl")?;
        Ok(Completed {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn program_available(&self, program: &str) -> bool {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    use super::*;
    use crate::domain::Manifest;

    /// A kubectl that echoes its arguments and acts on the name it is given.
    /// Written once, before any test spawns: a script still open for writing
    /// in one thread cannot be executed by another (ETXTBSY).
    fn fake_kubectl() -> PathBuf {
        static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
        SCRIPT
            .get_or_init(|| {
                let dir = std::env::temp_dir().join(format!("kube-mcp-{}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                let path = dir.join("kubectl");
                std::fs::write(
                    &path,
                    "#!/bin/sh\ncase \"$3\" in\n  slow) sleep 5 ;;\n  fail) echo boom >&2; exit 3 ;;\nesac\necho \"args=$*\"\necho \"ext=${KUBECTL_EXTERNAL_DIFF:-unset}\"\nif [ \"$1\" = diff ]; then cat; exit 1; fi\n",
                )
                .unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
                path
            })
            .clone()
    }

    #[tokio::test]
    async fn arguments_and_context_reach_kubectl_and_the_diff_variable_does_not() {
        // SAFETY: this test alone sets the variable; the runner must drop it for its child.
        unsafe { std::env::set_var("KUBECTL_EXTERNAL_DIFF", "evil") };
        let runner = ProcessRunner::new(fake_kubectl(), Some("kind-lab".into()));
        let completed = runner
            .run(&Invocation::describe("ns", "pods", "web").unwrap())
            .await
            .unwrap();
        assert!(completed.succeeded());
        assert!(
            completed
                .stdout
                .starts_with("args=--context kind-lab describe pods web -n ns"),
            "{}",
            completed.stdout
        );
        assert!(completed.stdout.contains("ext=unset"));
        assert!(completed.stderr.is_empty());
    }

    #[tokio::test]
    async fn the_manifest_goes_on_stdin_and_exit_one_is_not_an_error() {
        let runner = ProcessRunner::new(fake_kubectl(), None);
        let manifest = Manifest::parse("kind: ConfigMap\nmetadata: {name: c}\n", "ns").unwrap();
        let completed = runner
            .run(&Invocation::diff("ns", &manifest).unwrap())
            .await
            .unwrap();
        assert_eq!(completed.exit_code, Some(1));
        assert!(completed.stdout.starts_with("args=diff -f - -n ns"));
        assert!(completed.stdout.contains("kind: ConfigMap"));
    }

    #[tokio::test]
    async fn a_failing_run_keeps_its_code_and_stderr() {
        let runner = ProcessRunner::new(fake_kubectl(), None);
        let completed = runner
            .run(&Invocation::describe("ns", "pods", "fail").unwrap())
            .await
            .unwrap();
        assert_eq!(completed.exit_code, Some(3));
        assert_eq!(completed.stderr.trim(), "boom");
    }

    #[tokio::test]
    async fn a_slow_run_is_killed_at_the_timeout() {
        let runner =
            ProcessRunner::new(fake_kubectl(), None).with_timeout(Duration::from_millis(200));
        let error = runner
            .run(&Invocation::describe("ns", "pods", "slow").unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("did not finish"), "{error}");
    }

    #[tokio::test]
    async fn a_missing_binary_cannot_start() {
        let runner = ProcessRunner::new("/nonexistent/kubectl", None);
        let error = runner.run(&Invocation::version()).await.unwrap_err();
        assert!(error.to_string().contains("could not start"));
    }

    #[test]
    fn programs_are_looked_up_on_the_path() {
        let runner = ProcessRunner::new("kubectl", None);
        assert!(runner.program_available("sh"));
        assert!(!runner.program_available("no-such-program-for-this-test"));
    }
}
