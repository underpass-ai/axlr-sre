package domain

// Execution is one run of a task, with what the model cost.
type Execution struct {
	ID      string
	CostUSD float64
}
