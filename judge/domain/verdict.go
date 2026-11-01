package domain

// Verdict is Jev's choice about one expected fact.
type Verdict struct {
	Fact   Fact
	Choice Choice
}

// Mark renders the verdict for a table cell.
func (v Verdict) Mark() string {
	switch v.Choice.Option {
	case Stated:
		return "yes"
	case Contradicted:
		return "no"
	default:
		return "missing"
	}
}
