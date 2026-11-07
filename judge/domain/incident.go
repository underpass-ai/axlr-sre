package domain

import (
	"strings"
	"time"
)

// The incident states the control plane reports.
const (
	IncidentOpen      = "Open"
	IncidentDiagnosed = "Diagnosed"
	IncidentProposed  = "Proposed"
	IncidentEscalated = "Escalated"
	IncidentResolved  = "Resolved"
)

// HasDiagnosis reports a status past a diagnosis that stood: Diagnosed,
// or Proposed once the fix phase answered.
func HasDiagnosis(status string) bool {
	return status == IncidentDiagnosed || status == IncidentProposed
}

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
	// Proposal is the fix task's answer, once the incident is Proposed.
	Proposal string
	TaskIDs  []string
	// Tasks are TaskIDs with their bot, phase and cycle; empty from a
	// control plane older than the fix phase.
	Tasks     []IncidentTask
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

// CostSince is what the incident cost after earlier, the same incident
// read before. axlr-bots adds every execution of the incident's tasks and
// of the children they delegated to costUsd and never resets it, so a
// reopened incident still carries its earlier cycles. A read of another
// incident, or none (a new one), leaves the whole cost.
func (i Incident) CostSince(earlier Incident) float64 {
	if earlier.ID != i.ID {
		return i.CostUSD
	}
	return i.CostUSD - earlier.CostUSD
}

// FixTask is the fix task the control plane launched in the current
// cycle, or empty while it has not.
func (i Incident) FixTask() string {
	for n := len(i.Tasks) - 1; n >= 0; n-- {
		task := i.Tasks[n]
		if task.Phase == PhaseFix && task.Cycle == i.Cycle {
			return task.TaskID
		}
	}
	return ""
}
