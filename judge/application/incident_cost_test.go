package application

import (
	"context"
	"errors"
	"math"
	"testing"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func TestIncidentCostAddsTheChildren(t *testing.T) {
	cp := &fakeControlPlane{
		tasks: map[string]domain.Task{
			"t1": {ID: "t1", Children: []string{"c1", "c2"}},
			"c1": {ID: "c1", Children: []string{"g1"}},
			"c2": {ID: "c2"},
			"g1": {ID: "g1"},
		},
		executions: map[string][]domain.Execution{
			"c1": {{ID: "e1", CostUSD: 0.002}, {ID: "e2", CostUSD: 0.001}},
			"c2": {{ID: "e3", CostUSD: 0.003}},
			"g1": {{ID: "e4", CostUSD: 0.0005}},
		},
	}
	incident := domain.Incident{ID: "i", CostUSD: 0.004, TaskIDs: []string{"t0", "t1"}}
	cost, err := IncidentCost{ControlPlane: cp}.Of(context.Background(), incident)
	if err != nil {
		t.Fatal(err)
	}
	if math.Abs(cost-0.0105) > 1e-9 {
		t.Fatalf("cost %v", cost)
	}
	if cost, err := (IncidentCost{ControlPlane: cp}).Of(context.Background(), domain.Incident{CostUSD: 0.1}); err != nil || cost != 0.1 {
		t.Fatalf("no task: %v %v", cost, err)
	}
}

func TestIncidentCostStopsAtALoopAndReportsErrors(t *testing.T) {
	cp := &fakeControlPlane{
		tasks:      map[string]domain.Task{"t1": {ID: "t1", Children: []string{"t1"}}},
		executions: map[string][]domain.Execution{"t1": {{CostUSD: 1}}},
	}
	cost, err := IncidentCost{ControlPlane: cp}.Of(context.Background(), domain.Incident{TaskIDs: []string{"t1"}})
	if err != nil || cost != float64(maxDelegationDepth+1) {
		t.Fatalf("a task that is its own child is cut at the depth: %v %v", cost, err)
	}
	cp.fail = map[string]error{"executions": errors.New("down")}
	if _, err := (IncidentCost{ControlPlane: cp}).Of(context.Background(), domain.Incident{TaskIDs: []string{"t1"}}); err == nil {
		t.Fatal("an executions error surfaces")
	}
	cp.fail = map[string]error{"task": errors.New("down")}
	if _, err := (IncidentCost{ControlPlane: cp}).Of(context.Background(), domain.Incident{TaskIDs: []string{"t1"}}); err == nil {
		t.Fatal("a task error surfaces")
	}
	cp.fail = nil
	cp.tasks["t2"] = domain.Task{ID: "t2", Children: []string{"missing"}}
	if _, err := (IncidentCost{ControlPlane: cp}).Of(context.Background(), domain.Incident{TaskIDs: []string{"t2"}}); err == nil {
		t.Fatal("a missing child surfaces")
	}
}
