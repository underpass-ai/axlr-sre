package domain

import "time"

// Goals are a run's exit numbers (design §11, §12).
type Goals struct {
	// Name heads the goals' column in the tables.
	Name          string
	Scenarios     int
	Correct       int
	Duplicates    int
	OpenToSettled time.Duration
	CostPerRun    float64
}

// Phase0 was phase 0's bar: ten scenarios, kubectl on the host (§12.1).
var Phase0 = Goals{Name: "Phase 0", Scenarios: 10, Correct: 8, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}

// ReadNode is the bar a run on the read-only node sre-ro must clear,
// decided on 5 November 2026: the nine scenarios it can diagnose all fire
// (drift needs the write node), and at least eight are correct.
var ReadNode = Goals{Name: "Read node", Scenarios: 9, Correct: 8, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}
