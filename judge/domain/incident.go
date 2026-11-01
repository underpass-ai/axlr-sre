package domain

import (
	"strings"
	"time"
)

// The incident states the control plane reports.
const (
	IncidentOpen      = "Open"
	IncidentDiagnosed = "Diagnosed"
	IncidentEscalated = "Escalated"
	IncidentResolved  = "Resolved"
)

// Incident is what the control plane says of one alert group: as much of
// axlr-bots' aggregate as the judge reads.
type Incident struct {
	ID        string
	GroupKey  string
	Status    string
	Cycle     int
	Repeats   int
	CostUSD   float64
	Diagnosis string
	TaskIDs   []string
	OpenedAt  time.Time
	UpdatedAt time.Time
}

// IsFor reports whether the incident's group is the alert's: the group
// key carries alertname="…".
func (i Incident) IsFor(alert string) bool {
	return strings.Contains(i.GroupKey, `alertname="`+alert+`"`)
}

// Alert is the alert name inside the group key, or the key itself.
func (i Incident) Alert() string {
	const mark = `alertname="`
	start := strings.Index(i.GroupKey, mark)
	if start < 0 {
		return i.GroupKey
	}
	rest := i.GroupKey[start+len(mark):]
	if end := strings.Index(rest, `"`); end >= 0 {
		return rest[:end]
	}
	return rest
}

// Settled reports whether the current cycle has left Open.
func (i Incident) Settled() bool { return i.Status != IncidentOpen }

// CurrentTask is the triage task of the current cycle: the last one.
func (i Incident) CurrentTask() string {
	if len(i.TaskIDs) == 0 {
		return ""
	}
	return i.TaskIDs[len(i.TaskIDs)-1]
}
