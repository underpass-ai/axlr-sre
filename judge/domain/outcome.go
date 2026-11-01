package domain

// Outcome is what the verdicts say of the cause as a whole.
type Outcome string

const (
	// Correct: every cause fact stated.
	Correct Outcome = "correct"
	// Partly: some cause facts stated, none contradicted.
	Partly Outcome = "partly"
	// Wrong: a cause fact contradicted, or none stated, or no diagnosis.
	Wrong Outcome = "wrong"
)
