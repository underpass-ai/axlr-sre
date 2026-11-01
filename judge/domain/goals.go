package domain

import "time"

// Goals are phase 0's exit numbers (design §11, §12.1).
type Goals struct {
	Scenarios     int
	Correct       int
	Duplicates    int
	OpenToSettled time.Duration
	CostPerRun    float64
}

// Phase0 is the bar the run must clear.
var Phase0 = Goals{Scenarios: 10, Correct: 8, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}
