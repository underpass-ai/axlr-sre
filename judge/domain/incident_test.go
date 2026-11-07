package domain

import "testing"

func TestIncidentReadsItsGroup(t *testing.T) {
	i := Incident{GroupKey: `{}/{lab="true"}:{alertname="LabPodOOMKilled", namespace="shop"}`, Status: IncidentOpen, TaskIDs: []string{"t1", "t2"}}
	if !i.IsFor("LabPodOOMKilled") || i.IsFor("LabPod") || i.Alert() != "LabPodOOMKilled" {
		t.Fatalf("%q", i.Alert())
	}
	if i.Settled() || i.FixTask() != "" {
		t.Fatal("open, no fix task")
	}
	odd := Incident{GroupKey: "manual"}
	if odd.Alert() != "manual" {
		t.Fatal("a key without alertname is its own alert")
	}
	if (Incident{GroupKey: `alertname="X`}).Alert() != "X" {
		t.Fatal("an unclosed quote still names the alert")
	}
}

func TestIncidentFixTaskIsThisCycles(t *testing.T) {
	i := Incident{Cycle: 2, Tasks: []IncidentTask{
		{TaskID: "t1", Phase: PhaseTriage, Cycle: 1}, {TaskID: "f1", Phase: PhaseFix, Cycle: 1},
		{TaskID: "t2", Phase: PhaseTriage, Cycle: 2},
	}}
	if i.FixTask() != "" {
		t.Fatal("cycle 1's fix task is not cycle 2's")
	}
	i.Tasks = append(i.Tasks, IncidentTask{TaskID: "f2", Phase: PhaseFix, Cycle: 2})
	if i.FixTask() != "f2" {
		t.Fatal(i.FixTask())
	}
}

// On 5 November the OOM incident was reopened: $0.0075 in cycle 1, and
// costUsd read $0.0163 once cycle 2 was diagnosed.
func TestIncidentCostSinceLeavesTheEarlierCycles(t *testing.T) {
	before := Incident{ID: "inc-oom", Cycle: 1, CostUSD: 0.0075}
	after := Incident{ID: "inc-oom", Cycle: 2, CostUSD: 0.0163}
	if cost := after.CostSince(before); cost < 0.00879 || cost > 0.00881 {
		t.Fatalf("cycle 2 cost %v", cost)
	}
	if after.CostSince(Incident{}) != 0.0163 || after.CostSince(Incident{ID: "other", CostUSD: 1}) != 0.0163 {
		t.Fatal("a new incident's cost is all of it")
	}
}
