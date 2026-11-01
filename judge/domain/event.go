package domain

import "time"

// The incident events the judge reads from the bot's timeline.
const (
	EventIncidentOpened    = "incident.opened"
	EventIncidentReopened  = "incident.reopened"
	EventIncidentRepeated  = "incident.repeated"
	EventIncidentDiagnosed = "incident.diagnosed"
	EventIncidentEscalated = "incident.escalated"
	EventIncidentResolved  = "incident.resolved"
)

// Event is one entry of a bot's timeline, as much as the judge reads.
type Event struct {
	Type       string
	IncidentID string
	CreatedAt  time.Time
}

// Opens reports whether the event starts a cycle.
func (e Event) Opens() bool {
	return e.Type == EventIncidentOpened || e.Type == EventIncidentReopened
}

// Settles reports whether the event ends a cycle's triage.
func (e Event) Settles() bool {
	return e.Type == EventIncidentDiagnosed || e.Type == EventIncidentEscalated
}
