package domain

import "time"

// Result is one scenario's run: what fired, how fast, what it cost and
// what Jev made of the diagnosis.
type Result struct {
	Scenario   string
	Alert      string
	IncidentID string
	Cycle      int
	Status     string
	// Opened, Reopened and Repeated count the trigger's answers for the
	// incident since the injection.
	Opened, Reopened, Repeated int
	// Duplicates is how many incidents beyond the first the alert's group
	// opened since the injection.
	Duplicates int
	// OtherAlerts are alerts of other groups that opened since the
	// injection: side effects, not duplicates.
	OtherAlerts []string
	InjectedAt  time.Time
	SettledAt   time.Time
	// OpenToSettled comes from the bot's timeline, not from polling.
	OpenToSettled time.Duration
	// CostUSD is what the incident cost in this scenario's cycle: its
	// costUsd past what it was before the injection. axlr-bots counts the
	// delegated children in it; for a write-node scenario it is read once
	// the fix phase ended, so it holds the fix task's too.
	CostUSD   float64
	Repeats   int
	Diagnosis string
	// WriteNode marks a scenario only the write node handles: its Score is
	// fix's proposal's, not triage's diagnosis's.
	WriteNode bool      `json:",omitempty"`
	Proposal  *Proposal `json:",omitempty"`
	Score     Score
	// Error says why the scenario has no verdict: a timeout, a lab fault.
	Error string
}

// InjectToSettled is the time from the injection to the incident leaving
// Open, as the clock saw it.
func (r Result) InjectToSettled() time.Duration {
	if r.SettledAt.IsZero() {
		return 0
	}
	return r.SettledAt.Sub(r.InjectedAt)
}

// Fired reports whether the scenario's alert opened (or reopened) an
// incident.
func (r Result) Fired() bool { return r.IncidentID != "" }

// Outcome is the verdict on the cause, or on the proposal for a
// write-node scenario; a scenario without a diagnosis, or without a
// proposal when it needs one, is wrong.
func (r Result) Outcome() Outcome {
	if r.Error != "" || !HasDiagnosis(r.Status) {
		return Wrong
	}
	if r.WriteNode && (r.Proposal == nil || !r.Proposal.Completed()) {
		return Wrong
	}
	return r.Score.Outcome()
}
