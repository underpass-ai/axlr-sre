use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::IncidentDraft;
use crate::ports::IncidentSink;

const STDERR_CHARS: usize = 300;

/// `axlr-bots incident open|resolve` as a subprocess, every value its own
/// argument, with a timeout. The control plane's address comes from the
/// environment `axlr-bots` reads itself (`AXLR_BOTS_SOCKET`, `AXLR_BOTS_STATE`).
pub struct AxlrBotsCli {
    executable: PathBuf,
    timeout: Duration,
}

impl AxlrBotsCli {
    pub fn new(executable: PathBuf, timeout: Duration) -> Self {
        Self {
            executable,
            timeout,
        }
    }

    async fn run(&self, arguments: &[&str]) -> Result<(bool, String, String)> {
        let child = Command::new(&self.executable)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "{} could not be run: install axlr-bots or set AXLR_BOTS to its path",
                    self.executable.display()
                )
            })?;
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                anyhow!(
                    "axlr-bots {} did not finish in {} s",
                    arguments[..2].join(" "),
                    self.timeout.as_secs()
                )
            })?
            .context("axlr-bots")?;
        Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(STDERR_CHARS)
                .collect(),
        ))
    }
}

#[async_trait]
impl IncidentSink for AxlrBotsCli {
    async fn open(&self, draft: &IncidentDraft) -> Result<String> {
        let arguments = [
            "incident",
            "open",
            "--tenant",
            &draft.tenant,
            "--bot",
            &draft.bot,
            "--source",
            IncidentDraft::SOURCE,
            "--severity",
            draft.severity,
            "--group-key",
            &draft.group_key,
            "--fingerprints",
            &draft.fingerprint,
            "--title",
            &draft.title,
            "--objective",
            &draft.objective,
        ];
        let (success, stdout, stderr) = self.run(&arguments).await?;
        if !success {
            bail!("axlr-bots incident open failed: {stderr}");
        }
        let answer: serde_json::Value =
            serde_json::from_str(&stdout).context("axlr-bots incident open did not answer JSON")?;
        answer["incident"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("axlr-bots incident open answered without incident.id"))
    }

    async fn resolve(&self, incident_id: &str, reason: &str) -> Result<()> {
        let arguments = ["incident", "resolve", incident_id, "--reason", reason];
        let (success, _, stderr) = self.run(&arguments).await?;
        // Resolving twice fails naming the state it is in: that state is the one wanted.
        if success || stderr.contains("Resolved") {
            Ok(())
        } else {
            bail!("axlr-bots incident resolve failed: {stderr}")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    use super::*;
    use crate::domain::{Alert, ScopedAlert};

    /// One fake `axlr-bots` for every test, written once before any spawn so
    /// no child inherits a script open for writing. It appends its arguments,
    /// one per line, to `args` beside it and does what the `mode` file says.
    fn fake() -> PathBuf {
        static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
        SCRIPT
            .get_or_init(|| {
                let dir = std::env::temp_dir()
                    .join(format!("opsgenie-mirror-fake-{}", std::process::id()));
                std::fs::create_dir_all(&dir).unwrap();
                let script = dir.join("axlr-bots");
                std::fs::write(
                    &script,
                    "#!/bin/sh\nDIR=$(dirname \"$0\")\nfor a in \"$@\"; do echo \"$a\" >> \"$DIR/args\"; done\ncase \"$(cat \"$DIR/mode\")\" in\n  ok) echo '{\"created\": true, \"incident\": {\"id\": \"inc-7\"}}';;\n  noid) echo '{}';;\n  text) echo 'not json';;\n  paused) echo 'bot paused' >&2; exit 1;;\n  resolved) echo 'incident is Resolved' >&2; exit 1;;\n  missing) echo 'not found' >&2; exit 1;;\n  slow) sleep 5;;\nesac\n",
                )
                .unwrap();
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
                script
            })
            .clone()
    }

    /// The tests of this module run one at a time: the script's mode and
    /// args files are shared.
    async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        LOCK.lock().await
    }

    fn cli(mode: &str, timeout: Duration) -> AxlrBotsCli {
        let script = fake();
        let dir = script.parent().unwrap();
        std::fs::write(dir.join("mode"), mode).unwrap();
        std::fs::write(dir.join("args"), "").unwrap();
        AxlrBotsCli::new(script, timeout)
    }

    fn args() -> Vec<String> {
        std::fs::read_to_string(fake().parent().unwrap().join("args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn draft() -> IncidentDraft {
        let alert = Alert::read(&serde_json::json!({
            "id": "00000000-0000-4000-8000-000000000001",
            "tinyId": "1001",
            "message": "Disk full on node-1",
            "status": "open",
            "priority": "P2",
        }))
        .unwrap();
        IncidentDraft::for_alert(
            &ScopedAlert {
                alert,
                via: "Platform".into(),
            },
            "ops",
            "triage",
        )
    }

    #[tokio::test]
    async fn opens_with_every_value_in_its_own_argument_and_reads_the_id() {
        let _guard = lock().await;
        let cli = cli("ok", Duration::from_secs(5));
        assert_eq!(cli.open(&draft()).await.unwrap(), "inc-7");
        let args = args();
        assert_eq!(&args[..4], ["incident", "open", "--tenant", "ops"]);
        for pair in [
            ("--bot", "triage"),
            ("--severity", "sev2"),
            ("--source", "opsgenie"),
            ("--fingerprints", "1001"),
            (
                "--group-key",
                "opsgenie:00000000-0000-4000-8000-000000000001",
            ),
            ("--title", "Disk full on node-1 (Opsgenie #1001)"),
        ] {
            let at = args.iter().position(|a| a == pair.0).unwrap();
            assert_eq!(args[at + 1], pair.1);
        }
        assert!(args.join("\n").contains("Opsgenie alert #1001 (P2)"));
    }

    #[tokio::test]
    async fn an_answer_without_id_or_a_failure_is_explained() {
        let _guard = lock().await;
        let error = cli("noid", Duration::from_secs(5))
            .open(&draft())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("without incident.id"), "{error}");
        let error = cli("text", Duration::from_secs(5))
            .open(&draft())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("did not answer JSON"),
            "{error:#}"
        );
        let error = cli("paused", Duration::from_secs(5))
            .open(&draft())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("bot paused"), "{error}");
    }

    #[tokio::test]
    async fn resolves_and_takes_an_already_resolved_incident_as_done() {
        let _guard = lock().await;
        cli("noid", Duration::from_secs(5))
            .resolve("inc-7", "closed in Opsgenie")
            .await
            .unwrap();
        assert_eq!(
            args(),
            [
                "incident",
                "resolve",
                "inc-7",
                "--reason",
                "closed in Opsgenie"
            ]
        );
        cli("resolved", Duration::from_secs(5))
            .resolve("inc-7", "x")
            .await
            .unwrap();
        assert!(
            cli("missing", Duration::from_secs(5))
                .resolve("inc-7", "x")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_missing_or_hung_executable_fails_with_a_hint() {
        let _guard = lock().await;
        let dir = tempfile::tempdir().unwrap();
        let missing = AxlrBotsCli::new(dir.path().join("nope"), Duration::from_secs(1));
        let error = format!("{:#}", missing.open(&draft()).await.unwrap_err());
        assert!(error.contains("AXLR_BOTS"), "{error}");
        let hung = cli("slow", Duration::from_millis(300));
        let error = hung.open(&draft()).await.unwrap_err().to_string();
        assert!(error.contains("did not finish"), "{error}");
    }
}
