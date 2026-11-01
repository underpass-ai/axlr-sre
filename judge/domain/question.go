package domain

import "fmt"

// Limits keep one question inside one Jev request, as AXLR's axlr_judge
// and axlr-bots' Jev runtime do.
const (
	MaxStateBytes    = 24 << 10
	MaxQuestionBytes = 2000
	MaxOptions       = 16
	MaxOptionBytes   = 200
)

// The three answers Jev may give about one fact.
const (
	Stated       = "stated"
	Missing      = "missing"
	Contradicted = "contradicted"
)

// Question is one choice put to Jev: the state it reads, the question,
// the options it chooses among.
type Question struct {
	State    string
	Question string
	Options  []string
}

// NewFactQuestion asks whether a diagnosis states one expected fact. The
// diagnosis is cut to what one request holds; the fact travels in both
// the state and the question so Jev reads it beside the text.
func NewFactQuestion(fact Fact, diagnosis string) Question {
	const prefix = "EXPECTED FACT\n"
	head := prefix + fact.Line() + "\n\nDIAGNOSIS\n"
	room := MaxStateBytes - len(head)
	if len(diagnosis) > room {
		diagnosis = diagnosis[:room]
	}
	return Question{
		State: head + diagnosis,
		Question: fmt.Sprintf("Does the DIAGNOSIS state the EXPECTED FACT (%s)? Choose %q only if the diagnosis says every part of it, in any wording: an equivalent name, unit or number counts, a part left unsaid does not. Choose %q if the diagnosis says something incompatible with it. Choose %q if the diagnosis does not mention it.",
			fact.Name, Stated, Contradicted, Missing),
		Options: []string{Stated, Missing, Contradicted},
	}
}
