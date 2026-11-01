package domain

// Score is how a diagnosis did against the expected facts: one verdict
// per fact, the cause facts stated, the outcome, what judging cost.
type Score struct {
	Verdicts []Verdict
}

// NewScore derives the score from the verdicts.
func NewScore(verdicts []Verdict) Score { return Score{Verdicts: verdicts} }

// CauseTotal is how many facts make the cause (the fix is not one).
func (s Score) CauseTotal() int {
	n := 0
	for _, v := range s.Verdicts {
		if !v.Fact.IsFix() {
			n++
		}
	}
	return n
}

// CauseStated is how many cause facts the diagnosis states.
func (s Score) CauseStated() int {
	n := 0
	for _, v := range s.Verdicts {
		if !v.Fact.IsFix() && v.Choice.Option == Stated {
			n++
		}
	}
	return n
}

// Outcome applies the rule: a contradiction or nothing stated is wrong,
// everything stated is correct, the rest is partly.
func (s Score) Outcome() Outcome {
	total, stated := s.CauseTotal(), s.CauseStated()
	for _, v := range s.Verdicts {
		if !v.Fact.IsFix() && v.Choice.Option == Contradicted {
			return Wrong
		}
	}
	switch {
	case total == 0 || stated == 0:
		return Wrong
	case stated == total:
		return Correct
	default:
		return Partly
	}
}

// FixStated reports whether the diagnosis proposes the expected fix.
func (s Score) FixStated() bool {
	for _, v := range s.Verdicts {
		if v.Fact.IsFix() && v.Choice.Option == Stated {
			return true
		}
	}
	return false
}

// JevCostUSD is what the verdicts cost to ask.
func (s Score) JevCostUSD() float64 {
	cost := 0.0
	for _, v := range s.Verdicts {
		cost += v.Choice.CostUSD()
	}
	return cost
}

// MissingFacts names the cause facts not stated.
func (s Score) MissingFacts() []string {
	var names []string
	for _, v := range s.Verdicts {
		if !v.Fact.IsFix() && v.Choice.Option != Stated {
			names = append(names, v.Fact.Name)
		}
	}
	return names
}
