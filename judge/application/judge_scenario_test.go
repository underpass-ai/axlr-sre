package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

var t0 = time.Date(2026, 11, 1, 12, 0, 0, 0, time.UTC)

func settings() Settings {
	return Settings{Tenant: "ops", Bot: "triage", Timeout: time.Minute, Poll: 5 * time.Second, EventLimit: 100}
}

// world wires a scenario whose incident opens on injection and settles
// after a few polls.
type world struct {
	lab   *fakeLab
	cp    *fakeControlPlane
	clock *fakeClock
	judge *fakeJudge
	uc    JudgeScenario
	// afterSettle, for a write-node scenario, is the incident once the fix
	// phase ended, a poll after it settled; withoutFixTask settles it with
	// no fix task launched.
	afterSettle    func(domain.Incident) domain.Incident
	withoutFixTask bool
}

func newWorld() *world {
	w := &world{
		lab:   &fakeLab{expected: map[string]domain.Expected{"oom": oomExpected()}},
		cp:    &fakeControlPlane{executions: map[string][]domain.Execution{}},
		clock: &fakeClock{now: t0},
		judge: &fakeJudge{},
	}
	w.uc = JudgeScenario{Lab: w.lab, ControlPlane: w.cp, Clock: w.clock, Score: ScoreDiagnosis{Judge: w.judge}, Settings: settings()}
	return w
}

// opens makes the incident appear Open on injection and Diagnosed after
// two polls, with the timeline to match.
func (w *world) opens(id string, cycle int, status string) {
	polls := 0
	w.lab.onInject = func(string) {
		w.cp.set(domain.Incident{ID: id, GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, Cycle: cycle, TaskIDs: []string{"t1"}, OpenedAt: w.clock.Now().Add(2 * time.Second)})
		kind := domain.EventIncidentOpened
		if cycle > 1 {
			kind = domain.EventIncidentReopened
		}
		w.cp.event(kind, id, w.clock.Now().Add(2*time.Second))
	}
	w.clock.onSleep = func() {
		polls++
		if polls == 1 {
			w.cp.event(domain.EventIncidentRepeated, id, w.clock.Now())
		}
		if polls == 3 && w.afterSettle != nil {
			current, _ := w.cp.Incident(context.Background(), id)
			w.cp.set(w.afterSettle(current))
		}
		if polls == 2 {
			incident := domain.Incident{ID: id, GroupKey: groupKey("LabPodOOMKilled"), Status: status, Cycle: cycle, Repeats: 1, CostUSD: 0.005, Diagnosis: "cart is OOMKilled at 32Mi", TaskIDs: []string{"t1"}, OpenedAt: t0.Add(2 * time.Second)}
			incident.Tasks = []domain.IncidentTask{{TaskID: "t1", BotID: "triage", Phase: domain.PhaseTriage, Cycle: cycle}}
			if w.afterSettle != nil && !w.withoutFixTask && status == domain.IncidentDiagnosed {
				incident.TaskIDs = append(incident.TaskIDs, "task-fix")
				incident.Tasks = append(incident.Tasks, domain.IncidentTask{TaskID: "task-fix", BotID: "fix", Phase: domain.PhaseFix, Cycle: cycle})
			}
			w.cp.set(incident)
			settled := domain.EventIncidentDiagnosed
			if status == domain.IncidentEscalated {
				settled = domain.EventIncidentEscalated
			}
			w.cp.event(settled, id, w.clock.Now().Add(3*time.Second))
		}
	}
}

func TestJudgeScenarioMeasuresADiagnosedIncident(t *testing.T) {
	w := newWorld()
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if result.Error != "" || result.IncidentID != "inc-1" || result.Status != domain.IncidentDiagnosed || result.Outcome() != domain.Correct {
		t.Fatalf("%+v", result)
	}
	if result.Opened != 1 || result.Repeated != 1 || result.Reopened != 0 || result.Repeats != 1 {
		t.Fatalf("answers %+v", result)
	}
	if result.OpenToSettled != 6*time.Second || result.InjectToSettled() != 10*time.Second {
		t.Fatalf("open→settled %s inject→settled %s", result.OpenToSettled, result.InjectToSettled())
	}
	if result.CostUSD != 0.005 || result.Duplicates != 0 || len(result.OtherAlerts) != 0 {
		t.Fatalf("cost %v dup %d other %v", result.CostUSD, result.Duplicates, result.OtherAlerts)
	}
	if len(w.lab.cleared) != 1 || len(w.judge.asked) != 3 {
		t.Fatalf("cleared %v asked %d", w.lab.cleared, len(w.judge.asked))
	}
}

func TestJudgeScenarioCountsAReopeningAndTheSideEffects(t *testing.T) {
	w := newWorld()
	w.cp.set(domain.Incident{ID: "inc-old", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentResolved, Cycle: 1, CostUSD: 0.003, OpenedAt: t0.Add(-time.Hour)})
	w.cp.set(domain.Incident{ID: "inc-stale", GroupKey: groupKey("LabPodCrashLooping"), Status: domain.IncidentResolved, OpenedAt: t0.Add(-time.Hour)})
	w.opens("inc-old", 2, domain.IncidentDiagnosed)
	w.lab.onInject = func(s string) {
		w.cp.set(domain.Incident{ID: "inc-old", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, Cycle: 2, TaskIDs: []string{"t1"}, OpenedAt: t0.Add(-time.Hour)})
		w.cp.event(domain.EventIncidentReopened, "inc-old", w.clock.Now().Add(time.Second))
		w.cp.set(domain.Incident{ID: "inc-dup", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, Cycle: 1, OpenedAt: w.clock.Now().Add(time.Second)})
		w.cp.set(domain.Incident{ID: "inc-side", GroupKey: groupKey("LabPodCrashLooping"), Status: domain.IncidentOpen, Cycle: 1, OpenedAt: w.clock.Now().Add(time.Second)})
	}
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if result.IncidentID != "inc-old" || result.Cycle != 2 || result.Reopened != 1 || result.Opened != 0 {
		t.Fatalf("%+v", result)
	}
	if result.Duplicates != 1 || strings.Join(result.OtherAlerts, ",") != "LabPodCrashLooping" {
		t.Fatalf("dup %d other %v", result.Duplicates, result.OtherAlerts)
	}
	// costUsd reads 0.005 over both cycles; cycle 1 had cost 0.003.
	if result.CostUSD < 0.00199 || result.CostUSD > 0.00201 {
		t.Fatalf("the reopening's cost is its cycle's: %v", result.CostUSD)
	}
}

func TestJudgeScenarioTimesOutAndStillClears(t *testing.T) {
	w := newWorld()
	w.lab.onInject = func(string) {
		w.cp.set(domain.Incident{ID: "inc-1", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, Cycle: 1, OpenedAt: t0})
	}
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if result.Error != ErrTimeout.Error() || result.Outcome() != domain.Wrong || result.Fired() {
		t.Fatalf("%+v", result)
	}
	if len(w.lab.cleared) != 1 || w.clock.Now().Sub(t0) < time.Minute {
		t.Fatalf("cleared %v after %s", w.lab.cleared, w.clock.Now().Sub(t0))
	}
}

func TestJudgeScenarioEscalatedIsAnOutcomeNotAHang(t *testing.T) {
	w := newWorld()
	w.opens("inc-1", 1, domain.IncidentEscalated)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if result.Status != domain.IncidentEscalated || result.Outcome() != domain.Wrong || len(w.judge.asked) != 0 {
		t.Fatalf("%+v asked %d", result, len(w.judge.asked))
	}
}

func TestJudgeScenarioKeepsAndFinishesTheSoak(t *testing.T) {
	w := newWorld()
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", true)
	if err != nil || len(w.lab.cleared) != 0 {
		t.Fatalf("kept: %v %v", err, w.lab.cleared)
	}
	w.cp.set(domain.Incident{ID: "inc-1", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentDiagnosed, Cycle: 1, Repeats: 15, TaskIDs: []string{"t1"}, OpenedAt: t0})
	finished, err := w.uc.Finish(context.Background(), result)
	if err != nil {
		t.Fatal(err)
	}
	if finished.Repeats != 15 || len(w.lab.cleared) != 1 || finished.Duplicates != 0 {
		t.Fatalf("%+v cleared %v", finished, w.lab.cleared)
	}
	w.lab.clearErr = errors.New("kubectl wait failed")
	finished, err = w.uc.Finish(context.Background(), domain.Result{Scenario: "oom"})
	if err != nil || !strings.Contains(finished.Error, "clear: kubectl wait failed") {
		t.Fatalf("%v %+v", err, finished)
	}
	w.cp.fail = map[string]error{"incident": errors.New("down")}
	if _, err := w.uc.Finish(context.Background(), result); err == nil {
		t.Fatal("a control plane error surfaces")
	}
}

func TestJudgeScenarioSurfacesTheLabAndControlPlaneErrors(t *testing.T) {
	w := newWorld()
	if _, err := w.uc.Run(context.Background(), "nope", false); err == nil {
		t.Fatal("an unknown scenario")
	}
	w.lab.injErr = errors.New("inject.sh exited 1")
	if _, err := w.uc.Run(context.Background(), "oom", false); err == nil || !strings.Contains(err.Error(), "inject") {
		t.Fatalf("%v", err)
	}
	w.lab.injErr = nil
	w.cp.fail = map[string]error{"incidents": errors.New("down")}
	if _, err := w.uc.Run(context.Background(), "oom", false); err == nil {
		t.Fatal("the snapshot before the injection")
	}
	w = newWorld()
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	w.lab.clearErr = errors.New("nodes not Ready")
	w.cp.fail = map[string]error{"events": errors.New("no events")}
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil || !strings.Contains(result.Error, "no events") || !strings.Contains(result.Error, "clear: nodes not Ready") {
		t.Fatalf("%v %+v", err, result)
	}
	w = writeWorld("x", domain.IncidentProposed)
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	w.cp.fail = map[string]error{"task": errors.New("no task")}
	if result, _ := w.uc.Run(context.Background(), "oom", false); !strings.Contains(result.Error, "no task") {
		t.Fatalf("%+v", result)
	}
	w = newWorld()
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	w.judge.err = errors.New("TypeSafe refused the API key")
	if result, _ := w.uc.Run(context.Background(), "oom", false); !strings.Contains(result.Error, "refused") {
		t.Fatalf("%+v", result)
	}
	w = newWorld()
	w.lab.onInject = func(string) {
		w.cp.set(domain.Incident{ID: "inc-1", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, OpenedAt: t0})
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if result, _ := w.uc.Run(ctx, "oom", false); result.Error != context.Canceled.Error() {
		t.Fatalf("%+v", result)
	}
}
