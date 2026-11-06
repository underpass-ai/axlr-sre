package domain

import (
	"strings"
	"time"
)

// The task states that end a task, as the control plane names them.
const (
	TaskCompleted = "Completed"
	TaskFailed    = "Failed"
	TaskExpired   = "Expired"
	TaskCancelled = "Cancelled"
)

// TaskEnded reports a task status that no longer changes.
func TaskEnded(status string) bool {
	switch status {
	case TaskCompleted, TaskFailed, TaskExpired, TaskCancelled:
		return true
	}
	return false
}

// Proposal is what the fix bot answered for an incident of a write-node
// scenario: the task the judge submitted with triage's diagnosis, how it
// ended, its answer and what it cost. The control plane does not launch
// the fix phase yet (design §4), so the judge stands in for it.
type Proposal struct {
	TaskID      string
	Status      string
	Answer      string
	CostUSD     float64
	SubmittedAt time.Time
	EndedAt     time.Time
}

// Completed reports a proposal whose task ended with an answer.
func (p Proposal) Completed() bool {
	return p.Status == TaskCompleted && strings.TrimSpace(p.Answer) != ""
}

// Took is the time from the submission to the task's end.
func (p Proposal) Took() time.Duration {
	if p.SubmittedAt.IsZero() || p.EndedAt.IsZero() {
		return 0
	}
	return p.EndedAt.Sub(p.SubmittedAt)
}

// FixObjective is the fix task's objective: the incident, its alert and
// triage's diagnosis, and that nothing is applied.
func FixObjective(incident Incident) string {
	return "Incident " + incident.ID + " (alert " + incident.Alert() + ") was diagnosed by triage:\n\n" +
		strings.TrimSpace(incident.Diagnosis) +
		"\n\nPropose the fix and its rollback for this incident, as your runbook says; apply nothing."
}
