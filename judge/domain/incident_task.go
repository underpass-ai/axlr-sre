package domain

// The phases of an incident a task works on, as the control plane names
// them.
const (
	PhaseTriage = "triage"
	PhaseFix    = "fix"
)

// IncidentTask is one task that worked on an incident: which, by which
// bot, for which phase and in which cycle.
type IncidentTask struct {
	TaskID string
	BotID  string
	Phase  string
	Cycle  int
}
