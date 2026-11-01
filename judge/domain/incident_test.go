package domain

import "testing"

func TestIncidentReadsItsGroup(t *testing.T) {
	i := Incident{GroupKey: `{}/{lab="true"}:{alertname="LabPodOOMKilled", namespace="shop"}`, Status: IncidentOpen, TaskIDs: []string{"t1", "t2"}}
	if !i.IsFor("LabPodOOMKilled") || i.IsFor("LabPod") || i.Alert() != "LabPodOOMKilled" {
		t.Fatalf("%q", i.Alert())
	}
	if i.Settled() || i.CurrentTask() != "t2" {
		t.Fatal("open, current task t2")
	}
	odd := Incident{GroupKey: "manual"}
	if odd.Alert() != "manual" || odd.CurrentTask() != "" {
		t.Fatal("a key without alertname is its own alert")
	}
	if (Incident{GroupKey: `alertname="X`}).Alert() != "X" {
		t.Fatal("an unclosed quote still names the alert")
	}
}
