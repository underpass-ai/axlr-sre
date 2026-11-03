use std::io::Write;
use std::time::Duration;

use jiff::Timestamp;

use crate::application::MirrorIncidents;

/// The daemon's loop: one tick every interval, a line per thing that
/// happened, a failure logged and the loop going on. Stops on Ctrl-C or
/// SIGTERM. Nothing it writes carries the key.
pub struct MirrorDaemon {
    mirror: MirrorIncidents,
    interval: Duration,
    dry_run: bool,
}

impl MirrorDaemon {
    pub fn new(mirror: MirrorIncidents, interval: Duration, dry_run: bool) -> Self {
        Self {
            mirror,
            interval,
            dry_run,
        }
    }

    /// One tick. Returns whether it finished without failures.
    pub async fn run_once(&self, out: &mut dyn Write, verbose: bool) -> bool {
        let stamp = Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ");
        match self.mirror.tick(self.dry_run).await {
            Ok(report) => {
                for line in report.lines(verbose) {
                    let _ = writeln!(out, "{stamp} {line}");
                }
                report.failures.is_empty()
            }
            Err(error) => {
                let _ = writeln!(out, "{stamp} failed: {error:#}");
                false
            }
        }
    }

    /// Ticks until Ctrl-C or SIGTERM.
    pub async fn run_forever(&self, out: &mut dyn Write) {
        let _ = writeln!(
            out,
            "opsgenie-mirror: every {} s{}",
            self.interval.as_secs(),
            if self.dry_run { ", dry run" } else { "" }
        );
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        loop {
            self.run_once(out, false).await;
            let stop = async {
                match terminate.as_mut() {
                    Some(signal) => {
                        signal.recv().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(self.interval) => {}
                _ = tokio::signal::ctrl_c() => break,
                _ = stop => break,
            }
        }
        let _ = writeln!(out, "opsgenie-mirror: stopped");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::application::ReadAlerts;
    use crate::application::test_doubles::{FakeClient, FakeSink, FakeStore, listings, ours, page};
    use crate::domain::Scope;
    use crate::ports::OpsgenieClient;

    fn daemon(
        answers: Vec<(&'static str, Result<serde_json::Value, String>)>,
        dry_run: bool,
    ) -> MirrorDaemon {
        let client: Arc<dyn OpsgenieClient> = FakeClient::answering(answers);
        let mirror = MirrorIncidents::new(
            Arc::new(ReadAlerts::new(
                client,
                Scope::new(Some("platform"), None).unwrap(),
            )),
            Arc::new(FakeSink::default()),
            Arc::new(FakeStore::default()),
            "ops".into(),
            "triage".into(),
        );
        MirrorDaemon::new(mirror, Duration::from_secs(60), dry_run)
    }

    #[tokio::test]
    async fn a_tick_writes_its_lines_with_a_stamp() {
        let mut answers = listings();
        answers.push(page(vec![ours(1, "P1")]));
        let mut out = Vec::new();
        let ok = daemon(answers, true).run_once(&mut out, true).await;
        let text = String::from_utf8(out).unwrap();
        assert!(ok);
        assert!(
            text.contains("Z [dry-run] scope: 1 open, 0 excluded"),
            "{text}"
        );
        assert!(text.contains("first tick"));
    }

    #[tokio::test]
    async fn a_failure_is_logged_without_breaking() {
        let mut out = Vec::new();
        let daemon = daemon(
            vec![("/v2/teams", Err("Opsgenie answered 401".into()))],
            false,
        );
        assert!(!daemon.run_once(&mut out, false).await);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("failed: reading Opsgenie") && text.contains("401"),
            "{text}"
        );
    }
}
