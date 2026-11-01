package domain

import (
	"testing"
	"time"
)

func result(name string, outcome Outcome, cost float64, duplicates int) Result {
	option := Stated
	if outcome == Partly {
		option = Missing
	}
	r := Result{Scenario: name, IncidentID: "inc-" + name, Status: IncidentDiagnosed, CostUSD: cost, Duplicates: duplicates,
		InjectedAt: time.Unix(0, 0), SettledAt: time.Unix(90, 0), OpenToSettled: 40 * time.Second,
		Score: NewScore([]Verdict{verdict("a", Stated), verdict("b", option)})}
	if outcome == Wrong {
		r.Status = IncidentEscalated
	}
	return r
}

func TestRunTotalsAndGoals(t *testing.T) {
	var results []Result
	for i, o := range []Outcome{Correct, Correct, Correct, Correct, Correct, Correct, Correct, Correct, Partly, Wrong} {
		results = append(results, result(string(rune('j'-i)), o, 0.01, 0))
	}
	run := NewRun(results)
	if run.Results[0].Scenario != "a" {
		t.Fatal("results are sorted by scenario")
	}
	if run.Fired() != 10 || run.Correct() != 8 || run.Partly() != 1 || run.Duplicates() != 0 {
		t.Fatalf("fired %d correct %d partly %d dup %d", run.Fired(), run.Correct(), run.Partly(), run.Duplicates())
	}
	if run.CostUSD() < 0.0999 || run.CostUSD() > 0.1001 || run.JevCostUSD() <= 0 {
		t.Fatalf("cost %v jev %v", run.CostUSD(), run.JevCostUSD())
	}
	if !run.Passes(Phase0) {
		t.Fatal("8/10 correct, no duplicates, under $1 passes")
	}
	results[0].Duplicates = 1
	if NewRun(results).Passes(Phase0) {
		t.Fatal("a duplicate fails")
	}
	results[0].Duplicates = 0
	results[1].CostUSD = 2
	if NewRun(results).Passes(Phase0) {
		t.Fatal("over budget fails")
	}
	if NewRun(results[:9]).Passes(Phase0) {
		t.Fatal("nine scenarios are not ten")
	}
}

func TestResultOutcomeNeedsADiagnosis(t *testing.T) {
	r := result("x", Correct, 0, 0)
	if r.Outcome() != Correct || r.InjectToSettled() != 90*time.Second || !r.Fired() {
		t.Fatalf("%+v", r)
	}
	r.Error = "timeout"
	if r.Outcome() != Wrong {
		t.Fatal("an error is wrong")
	}
	if (Result{}).InjectToSettled() != 0 || (Result{}).Fired() {
		t.Fatal("nothing settled, nothing fired")
	}
}
