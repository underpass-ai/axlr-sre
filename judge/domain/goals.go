package domain

import "time"

// Goals are a run's exit numbers (design §11, §12).
type Goals struct {
	// Name heads the goals' column in the tables.
	Name string
	// Judged is what Jev reads: the causes, or fix's proposals.
	Judged        string
	Scenarios     int
	Correct       int
	Duplicates    int
	OpenToSettled time.Duration
	CostPerRun    float64
}

// Phase0 was phase 0's bar: ten scenarios, kubectl on the host (§12.1).
var Phase0 = Goals{Name: "Phase 0", Judged: "causes", Scenarios: 10, Correct: 8, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}

// ReadNode is the bar a run on the read-only node sre-ro must clear,
// decided on 5 November 2026: the nine scenarios it can diagnose all fire
// (drift needs the write node), and at least eight are correct.
var ReadNode = Goals{Name: "Read node", Judged: "causes", Scenarios: 9, Correct: 8, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}

// WriteNode is the bar of the write node sre-rw's scenarios, set on 6
// November 2026 while drift is its only one: every one fires, and fix's
// proposal states every fact, its diff and rollback included.
var WriteNode = Goals{Name: "Write node", Judged: "proposals", Scenarios: 1, Correct: 1, Duplicates: 0, OpenToSettled: 3 * time.Minute, CostPerRun: 1}
