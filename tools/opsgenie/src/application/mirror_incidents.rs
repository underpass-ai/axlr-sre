use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::ReadAlerts;
use crate::domain::{IncidentDraft, MirrorPlan, MirrorState, ScopedAlert};
use crate::ports::{IncidentSink, StateStore};

/// What one tick did, or would do in a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickReport {
    pub dry_run: bool,
    pub in_scope: usize,
    pub excluded: usize,
    pub complete: bool,
    /// `Some(n)`: this tick was the baseline of n alerts.
    pub baseline: Option<usize>,
    pub opened: Vec<(String, String)>,
    pub resolved: Vec<String>,
    pub forgotten: usize,
    pub planned_open: Vec<String>,
    pub planned_resolve: Vec<String>,
    pub resolution_skipped: bool,
    pub failures: Vec<String>,
}

impl TickReport {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "dryRun": self.dry_run,
            "inScope": self.in_scope,
            "excluded": self.excluded,
            "complete": self.complete,
            "baseline": self.baseline,
            "opened": self.opened.iter().map(|(tiny, inc)| json!({"alert": tiny, "incident": inc})).collect::<Vec<_>>(),
            "resolved": self.resolved,
            "forgotten": self.forgotten,
            "plannedOpen": self.planned_open,
            "plannedResolve": self.planned_resolve,
            "resolutionSkipped": self.resolution_skipped,
            "failures": self.failures,
        })
    }

    /// One line per thing that happened, for the daemon's log; nothing on
    /// a quiet tick unless `verbose`.
    #[must_use]
    pub fn lines(&self, verbose: bool) -> Vec<String> {
        let prefix = if self.dry_run { "[dry-run] " } else { "" };
        let mut lines = Vec::new();
        if verbose || self.baseline.is_some() {
            lines.push(format!(
                "{prefix}scope: {} open, {} excluded{}",
                self.in_scope,
                self.excluded,
                if self.complete {
                    ""
                } else {
                    " (listing incomplete)"
                }
            ));
        }
        if let Some(n) = self.baseline {
            lines.push(format!(
                "{prefix}first tick: {n} open alerts noted as the baseline, none opened"
            ));
        }
        for tiny in &self.planned_open {
            lines.push(format!("{prefix}would open an incident for alert #{tiny}"));
        }
        for incident in &self.planned_resolve {
            lines.push(format!("{prefix}would resolve incident {incident}"));
        }
        for (tiny, incident) in &self.opened {
            lines.push(format!("alert #{tiny}: opened incident {incident}"));
        }
        for incident in &self.resolved {
            lines.push(format!(
                "resolved incident {incident}: alert closed in Opsgenie"
            ));
        }
        if self.forgotten > 0 && (verbose || self.dry_run) {
            lines.push(format!(
                "{prefix}{} baseline alerts closed and forgotten",
                self.forgotten
            ));
        }
        if self.resolution_skipped {
            lines.push(format!(
                "{prefix}listing incomplete: nothing resolved this tick"
            ));
        }
        for failure in &self.failures {
            lines.push(format!("failed: {failure}"));
        }
        lines
    }
}

/// One tick of the mirror: a new alert of the scope opens an incident, an
/// alert no longer open resolves the one it opened. The first tick is a
/// baseline. A failure reading Opsgenie or the state aborts the tick with
/// nothing touched; a failure with one incident is noted and retried next
/// tick.
pub struct MirrorIncidents {
    read: Arc<ReadAlerts>,
    sink: Arc<dyn IncidentSink>,
    store: Arc<dyn StateStore>,
    tenant: String,
    bot: String,
}

impl MirrorIncidents {
    pub fn new(
        read: Arc<ReadAlerts>,
        sink: Arc<dyn IncidentSink>,
        store: Arc<dyn StateStore>,
        tenant: String,
        bot: String,
    ) -> Self {
        Self {
            read,
            sink,
            store,
            tenant,
            bot,
        }
    }

    pub async fn tick(&self, dry_run: bool) -> Result<TickReport> {
        let mut state = self.store.load().context("the mirror's state")?;
        let open = self.read.open().await.context("reading Opsgenie")?;
        let plan = state.plan(&open);
        let mut report = TickReport {
            dry_run,
            in_scope: open.alerts.len(),
            excluded: open.excluded,
            complete: open.complete,
            resolution_skipped: plan.resolution_skipped,
            ..TickReport::default()
        };
        if dry_run {
            report.baseline = plan.baseline.as_ref().map(Vec::len);
            report.planned_open = plan.open.iter().map(|s| s.alert.tiny_id.clone()).collect();
            report.planned_resolve = plan.resolve.into_iter().map(|(_, inc)| inc).collect();
            report.forgotten = plan.forget.len();
            return Ok(report);
        }
        if let Some(ids) = &plan.baseline {
            state.record_baseline(ids);
            report.baseline = Some(ids.len());
            self.save(&state, &mut report);
            return Ok(report);
        }
        self.apply(plan, &mut state, &mut report).await;
        Ok(report)
    }

    async fn apply(&self, plan: MirrorPlan, state: &mut MirrorState, report: &mut TickReport) {
        for scoped in plan.open {
            match self.open_one(&scoped).await {
                Ok(incident) => {
                    state.record_opened(&scoped.alert.id, &incident);
                    self.save(state, report);
                    report.opened.push((scoped.alert.tiny_id.clone(), incident));
                }
                Err(error) => report
                    .failures
                    .push(format!("open #{}: {error:#}", scoped.alert.tiny_id)),
            }
        }
        for (alert, incident) in plan.resolve {
            match self
                .sink
                .resolve(&incident, MirrorPlan::RESOLVE_REASON)
                .await
            {
                Ok(()) => {
                    state.forget(&alert);
                    self.save(state, report);
                    report.resolved.push(incident);
                }
                Err(error) => report
                    .failures
                    .push(format!("resolve {incident}: {error:#}")),
            }
        }
        if !plan.forget.is_empty() {
            for alert in &plan.forget {
                state.forget(alert);
            }
            report.forgotten = plan.forget.len();
            self.save(state, report);
        }
    }

    async fn open_one(&self, scoped: &ScopedAlert) -> Result<String> {
        let draft = IncidentDraft::for_alert(scoped, &self.tenant, &self.bot);
        self.sink.open(&draft).await
    }

    fn save(&self, state: &MirrorState, report: &mut TickReport) {
        if let Err(error) = self.store.save(state) {
            report.failures.push(format!("save state: {error:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_doubles::{
        FakeClient, FakeSink, FakeStore, listings, ours, page, theirs,
    };
    use super::*;
    use crate::domain::Scope;
    use crate::ports::OpsgenieClient;

    struct World {
        client: Arc<FakeClient>,
        sink: Arc<FakeSink>,
        store: Arc<FakeStore>,
    }

    impl World {
        fn new(sink: FakeSink, store: Arc<FakeStore>) -> Self {
            let mut answers = listings();
            answers.push(page(vec![ours(1, "P1"), theirs(9)]));
            Self {
                client: FakeClient::answering(answers),
                sink: Arc::new(sink),
                store,
            }
        }

        fn alerts(&self, items: Vec<Value>) {
            let mut answers = listings();
            answers.push(page(items));
            self.client.set(answers);
        }

        fn mirror(&self) -> MirrorIncidents {
            let client: Arc<dyn OpsgenieClient> = self.client.clone();
            MirrorIncidents::new(
                Arc::new(ReadAlerts::new(
                    client,
                    Scope::new(Some("platform"), None).unwrap(),
                )),
                self.sink.clone(),
                self.store.clone(),
                "ops".into(),
                "triage".into(),
            )
        }
    }

    fn seeded() -> Arc<FakeStore> {
        FakeStore::with(MirrorState::restore(true, Default::default()))
    }

    #[tokio::test]
    async fn the_first_tick_is_a_baseline_and_opens_nothing() {
        let world = World::new(FakeSink::default(), Arc::new(FakeStore::default()));
        let report = world.mirror().tick(false).await.unwrap();
        assert_eq!(report.baseline, Some(1));
        assert!(world.sink.opened.lock().unwrap().is_empty());
        assert!(world.store.state().baseline_done());
        assert_eq!((report.in_scope, report.excluded), (1, 1));
        assert!(report.lines(false).iter().any(|l| l.contains("first tick")));
        assert_eq!(report.to_json()["baseline"], 1);
    }

    #[tokio::test]
    async fn a_new_alert_opens_an_incident_and_its_closing_resolves_it() {
        let world = World::new(FakeSink::default(), Arc::new(FakeStore::default()));
        let mirror = world.mirror();
        mirror.tick(false).await.unwrap();
        world.alerts(vec![ours(1, "P1"), ours(2, "P3"), theirs(9)]);
        let report = mirror.tick(false).await.unwrap();
        assert_eq!(report.opened, [("2".to_string(), "inc-2".to_string())]);
        let draft = world.sink.opened.lock().unwrap()[0].clone();
        assert_eq!(
            draft.group_key,
            "opsgenie:00000000-0000-4000-8000-000000000002"
        );
        assert_eq!(draft.severity, "sev3");
        assert_eq!(draft.tenant, "ops");
        assert_eq!(world.store.state().seen().len(), 2);
        assert!(report.lines(false)[0].contains("opened incident inc-2"));

        world.alerts(vec![]);
        let report = mirror.tick(false).await.unwrap();
        assert_eq!(report.resolved, ["inc-2"]);
        assert_eq!(report.forgotten, 1);
        assert_eq!(
            world.sink.resolved.lock().unwrap()[0],
            ("inc-2".to_string(), "closed in Opsgenie".to_string())
        );
        assert!(world.store.state().seen().is_empty());
        let lines = report.lines(true);
        assert!(lines.iter().any(|l| l.contains("resolved incident inc-2")));
        assert!(lines.iter().any(|l| l.contains("1 baseline alerts closed")));
        assert_eq!(report.to_json()["resolved"][0], "inc-2");
    }

    #[tokio::test]
    async fn a_dry_run_calls_nothing_and_saves_nothing() {
        let world = World::new(FakeSink::default(), seeded());
        let report = world.mirror().tick(true).await.unwrap();
        assert!(report.dry_run);
        assert_eq!(report.planned_open, ["1"]);
        assert!(world.sink.opened.lock().unwrap().is_empty());
        assert_eq!(*world.store.saves.lock().unwrap(), 0);
        assert!(report.lines(false)[0].starts_with("[dry-run] would open"));

        let first = World::new(FakeSink::default(), Arc::new(FakeStore::default()));
        let report = first.mirror().tick(true).await.unwrap();
        assert_eq!(report.baseline, Some(1));
        assert!(!first.store.state().baseline_done());

        let mut state = MirrorState::restore(true, Default::default());
        state.record_opened("gone", "inc-gone");
        let world = World::new(FakeSink::default(), FakeStore::with(state));
        let report = world.mirror().tick(true).await.unwrap();
        assert_eq!(report.planned_resolve, ["inc-gone"]);
        assert!(
            report
                .lines(false)
                .iter()
                .any(|l| l.contains("would resolve incident inc-gone"))
        );
        assert_eq!(report.to_json()["plannedResolve"][0], "inc-gone");
    }

    #[tokio::test]
    async fn a_failed_open_is_noted_and_retried_next_tick() {
        let world = World::new(
            FakeSink {
                open_fails: true,
                ..FakeSink::default()
            },
            seeded(),
        );
        let report = world.mirror().tick(false).await.unwrap();
        assert!(
            report.failures[0].starts_with("open #1"),
            "{:?}",
            report.failures
        );
        assert!(world.store.state().seen().is_empty());
        assert!(report.lines(false)[0].starts_with("failed: open #1"));
        assert_eq!(report.to_json()["failures"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_resolve_keeps_the_incident() {
        let mut state = MirrorState::restore(true, Default::default());
        state.record_opened("00000000-0000-4000-8000-000000000001", "inc-1");
        let world = World::new(
            FakeSink {
                resolve_fails: true,
                ..FakeSink::default()
            },
            FakeStore::with(state),
        );
        world.alerts(vec![]);
        let report = world.mirror().tick(false).await.unwrap();
        assert!(report.failures[0].starts_with("resolve inc-1"));
        assert_eq!(world.store.state().seen().len(), 1);
    }

    #[tokio::test]
    async fn an_incomplete_listing_resolves_nothing() {
        let mut state = MirrorState::restore(true, Default::default());
        state.record_opened("gone", "inc-gone");
        let world = World::new(FakeSink::default(), FakeStore::with(state));
        let full: Vec<Value> = (0..crate::domain::PAGE_SIZE)
            .map(|n| ours(n, "P2"))
            .collect();
        world.alerts(full);
        let report = world.mirror().tick(false).await.unwrap();
        assert!(report.resolution_skipped);
        assert!(report.resolved.is_empty());
        assert_eq!(
            world.store.state().seen().get("gone").unwrap().as_deref(),
            Some("inc-gone")
        );
        assert!(
            report
                .lines(false)
                .iter()
                .any(|l| l.contains("listing incomplete"))
        );
    }

    #[tokio::test]
    async fn an_unreadable_opsgenie_or_state_aborts_the_tick_untouched() {
        let world = World::new(FakeSink::default(), Arc::new(FakeStore::default()));
        world
            .client
            .set(vec![("/v2/teams", Err("Opsgenie answered 401".into()))]);
        let error = world.mirror().tick(false).await.unwrap_err().to_string();
        assert!(error.contains("reading Opsgenie"), "{error}");
        assert_eq!(*world.store.saves.lock().unwrap(), 0);

        let world = World::new(
            FakeSink::default(),
            Arc::new(FakeStore {
                load_fails: true,
                ..FakeStore::default()
            }),
        );
        let error = world.mirror().tick(false).await.unwrap_err();
        assert!(format!("{error:#}").contains("state unreadable"));
        assert!(world.client.paths().is_empty());
    }

    #[tokio::test]
    async fn a_failed_save_is_noted() {
        let world = World::new(
            FakeSink::default(),
            Arc::new(FakeStore {
                save_fails: true,
                ..FakeStore::default()
            }),
        );
        let report = world.mirror().tick(false).await.unwrap();
        assert!(report.failures[0].contains("disk full"));
    }
}
