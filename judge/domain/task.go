package domain

import "time"

// Task is as much of a control-plane task as the judge reads: its state,
// whom it delegated to, which executions ran it, when it was created.
type Task struct {
	ID           string
	Status       string
	Children     []string
	ExecutionIDs []string
	CreatedAt    time.Time
}
