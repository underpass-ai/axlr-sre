use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::Invocation;
use crate::ports::{AwsRunner, Completed};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// Spawns `aws`. Its stdin is null and its stdout and stderr are pipes,
/// never inherited: this process's own stdio is the MCP protocol. Every run
/// with a region gets `--region`, `--output json`, `--no-cli-pager`, the
/// connect and read timeouts and, when configured, `--profile`; the pager
/// variable is emptied. Nothing else of the environment is touched, since a
/// node's role (IRSA) and an SSO session both live there. A run that
/// outlives the timeout is killed.
pub struct ProcessRunner {
    aws: PathBuf,
    profile: Option<String>,
    timeout: Duration,
}

impl ProcessRunner {
    pub fn new(aws: impl AsRef<Path>, profile: Option<String>) -> Self {
        Self {
            aws: aws.as_ref().to_path_buf(),
            profile,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn command(&self, invocation: &Invocation) -> Command {
        let mut command = Command::new(&self.aws);
        if let Some(region) = invocation.region() {
            command.args(Invocation::global_args(region, self.profile.as_deref()));
        }
        command
            .args(invocation.args())
            .env("AWS_PAGER", "")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
}

#[async_trait]
impl AwsRunner for ProcessRunner {
    async fn run(&self, invocation: &Invocation) -> Result<Completed> {
        let child = self
            .command(invocation)
            .spawn()
            .with_context(|| format!("could not start {}", self.aws.display()))?;
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                anyhow!(
                    "aws {} did not finish in {}s and was killed",
                    invocation.verb(),
                    self.timeout.as_secs()
                )
            })?
            .context("waiting for aws")?;
        Ok(Completed {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    use super::*;

    /// One fake `aws` for every test, written once before any spawn. It
    /// echoes its arguments as JSON, or does what a keyword among them says.
    fn fake() -> PathBuf {
        static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
        SCRIPT
            .get_or_init(|| {
                let dir = std::env::temp_dir().join(format!("aws-mcp-fake-{}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                let script = dir.join("aws");
                std::fs::write(
                    &script,
                    "#!/bin/sh\ncase \"$*\" in\n  --version) echo 'aws-cli/2.36.24 Python/3.14.6'; exit 0;;\n  *slow*) sleep 5;;\n  *fail*) echo 'An error occurred (AccessDeniedException) when calling the X operation: no' >&2; exit 254;;\nesac\nprintf '{\"args\": \"%s\", \"pager\": \"%s\"}\\n' \"$*\" \"${AWS_PAGER-unset}\"\n",
                )
                .unwrap();
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
                script
            })
            .clone()
    }

    #[tokio::test]
    async fn the_global_flags_and_the_profile_precede_the_arguments() {
        let runner = ProcessRunner::new(fake(), Some("demo".into()));
        let completed = runner
            .run(&Invocation::caller_identity("eu-west-1"))
            .await
            .unwrap();
        assert!(completed.succeeded());
        let answer: serde_json::Value = serde_json::from_str(&completed.stdout).unwrap();
        let args = answer["args"].as_str().unwrap();
        assert!(
            args.starts_with("--region eu-west-1 --output json --no-cli-pager"),
            "{args}"
        );
        assert!(
            args.contains("--profile demo sts get-caller-identity"),
            "{args}"
        );
        assert_eq!(answer["pager"], "");
        let version = runner.run(&Invocation::version()).await.unwrap();
        assert!(version.stdout.starts_with("aws-cli/2"));
        let no_profile = ProcessRunner::new(fake(), None)
            .run(&Invocation::caller_identity("us-east-1"))
            .await
            .unwrap();
        assert!(!no_profile.stdout.contains("--profile"));
    }

    #[tokio::test]
    async fn a_failure_keeps_its_stderr_and_a_hung_run_is_killed() {
        let runner = ProcessRunner::new(fake(), None);
        let failed = runner
            .run(&Invocation::function_configuration("eu-west-1", "fail"))
            .await
            .unwrap();
        assert_eq!(failed.exit_code, Some(254));
        assert!(failed.stderr.contains("AccessDeniedException"));
        let hung = ProcessRunner::new(fake(), None).with_timeout(Duration::from_millis(300));
        let error = hung
            .run(&Invocation::function_configuration("eu-west-1", "slow"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("did not finish"), "{error}");
        let missing = ProcessRunner::new("/nope/aws", None)
            .run(&Invocation::version())
            .await
            .unwrap_err();
        assert!(format!("{missing:#}").contains("could not start"));
    }
}
