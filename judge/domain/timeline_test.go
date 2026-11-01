package domain

import (
	"testing"
	"time"
)

func TestTimelineOpenToSettledAndCounts(t *testing.T) {
	t0 := time.Date(2026, 11, 1, 10, 0, 0, 0, time.UTC)
	at := func(s int) time.Time { return t0.Add(time.Duration(s) * time.Second) }
	timeline := Timeline{
		{Type: EventIncidentOpened, IncidentID: "a", CreatedAt: at(0)},
		{Type: EventIncidentDiagnosed, IncidentID: "a", CreatedAt: at(30)},
		{Type: EventIncidentResolved, IncidentID: "a", CreatedAt: at(100)},
		{Type: EventIncidentReopened, IncidentID: "a", CreatedAt: at(200)},
		{Type: EventIncidentRepeated, IncidentID: "a", CreatedAt: at(210)},
		{Type: EventIncidentRepeated, IncidentID: "b", CreatedAt: at(215)},
		{Type: EventIncidentEscalated, IncidentID: "a", CreatedAt: at(260)},
	}
	own := timeline.Of("a", at(150))
	if len(own) != 3 || own.Count(EventIncidentRepeated) != 1 || own.Count(EventIncidentReopened) != 1 {
		t.Fatalf("%+v", own)
	}
	if got := own.OpenToSettled(); got != 60*time.Second {
		t.Fatalf("reopen → escalated %s", got)
	}
	if got := timeline.Of("a", t0).OpenToSettled(); got != 30*time.Second {
		t.Fatalf("first cycle %s", got)
	}
	if got := timeline.Of("b", t0).OpenToSettled(); got != 0 {
		t.Fatalf("no pair %s", got)
	}
}
