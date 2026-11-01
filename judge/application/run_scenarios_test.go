package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func TestRunScenariosKeepsEachResultAndSoaksOne(t *testing.T) {
	w := newWorld()
	w.lab.expected["drift"] = domain.Expected{Scenario: "drift", Alert: "LabDeploymentDrift", Facts: []domain.Fact{{Name: "object", Value: "web"}}}
	n := 0
	w.lab.onInject = func(scenario string) {
		n++
		id := "inc-" + scenario
		alert := w.lab.expected[scenario].Alert
		w.cp.set(domain.Incident{ID: id, GroupKey: groupKey(alert), Status: domain.IncidentDiagnosed, Cycle: 1, Diagnosis: "d", TaskIDs: []string{"t1"}, OpenedAt: w.clock.Now().Add(time.Second)})
		w.cp.event(domain.EventIncidentOpened, id, w.clock.Now().Add(time.Second))
		w.cp.event(domain.EventIncidentDiagnosed, id, w.clock.Now().Add(31*time.Second))
	}
	var kept []string
	var said []string
	uc := RunScenarios{
		Scenario: w.uc, Clock: w.clock,
		Soak: Soak{Scenario: "oom", For: 10 * time.Minute},
		Keep: func(r domain.Result) error { kept = append(kept, r.Scenario); return nil },
		Say:  func(line string) { said = append(said, line) },
	}
	run, err := uc.Run(context.Background(), []string{"oom", "drift"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(kept, ",") != "drift,oom" || strings.Join(w.lab.cleared, ",") != "drift,oom" {
		t.Fatalf("kept %v cleared %v", kept, w.lab.cleared)
	}
	if run.Fired() != 2 || run.Correct() != 2 || w.clock.Now().Sub(t0) < 10*time.Minute {
		t.Fatalf("fired %d correct %d after %s", run.Fired(), run.Correct(), w.clock.Now().Sub(t0))
	}
	if !strings.Contains(strings.Join(said, "\n"), "holding") {
		t.Fatalf("said %v", said)
	}
}

func TestRunScenariosStopsOnErrors(t *testing.T) {
	w := newWorld()
	uc := RunScenarios{Scenario: w.uc, Clock: w.clock}
	if _, err := uc.Run(context.Background(), []string{"nope"}); err == nil {
		t.Fatal("an unknown scenario stops the run")
	}
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	uc.Keep = func(domain.Result) error { return errors.New("disk full") }
	if _, err := uc.Run(context.Background(), []string{"oom"}); err == nil || err.Error() != "disk full" {
		t.Fatalf("%v", err)
	}
	w = newWorld()
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	w.cp.fail = map[string]error{"incident": errors.New("down")}
	uc = RunScenarios{Scenario: w.uc, Clock: w.clock, Soak: Soak{Scenario: "oom", For: time.Minute}}
	if _, err := uc.Run(context.Background(), []string{"oom"}); err == nil {
		t.Fatal("the soak's finish error surfaces")
	}
	w = newWorld()
	w.lab.onInject = func(string) {
		w.cp.set(domain.Incident{ID: "inc-1", GroupKey: groupKey("LabPodOOMKilled"), Status: domain.IncidentOpen, OpenedAt: t0})
	}
	uc = RunScenarios{Scenario: w.uc, Clock: w.clock, Soak: Soak{Scenario: "oom", For: time.Minute}}
	run, err := uc.Run(context.Background(), []string{"oom"})
	if err != nil || len(run.Results) != 1 || run.Results[0].Error == "" || len(w.lab.cleared) != 1 {
		t.Fatalf("a soak that never settled is kept as a failure and cleared: %v %+v %v", err, run.Results, w.lab.cleared)
	}
	if !strings.Contains(summary(run.Results[0]), "no verdict") {
		t.Fatal(summary(run.Results[0]))
	}
}
