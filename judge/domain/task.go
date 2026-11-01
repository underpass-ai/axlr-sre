package domain

// Task is as much of a control-plane task as the judge reads: whom it
// delegated to, which executions ran it.
type Task struct {
	ID           string
	Children     []string
	ExecutionIDs []string
}
