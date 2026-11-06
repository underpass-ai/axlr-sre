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
// scenario: the fix task the control plane launched once triage
// diagnosed it (design §4), how it ended, its answer (the incident's
// proposal) and what it cost. SubmittedAt is when the control plane
// created the task, EndedAt when the judge saw the incident leave
// Diagnosed.
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
