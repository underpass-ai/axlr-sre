package domain

// Execution is one run of a task, with what the model cost and the
// answer it ended with.
type Execution struct {
	ID      string
	CostUSD float64
	Answer  string
}
