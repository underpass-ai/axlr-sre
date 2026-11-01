package domain

import (
	"strings"
	"testing"
)

func TestNewFactQuestionCarriesTheFactAndCutsTheDiagnosis(t *testing.T) {
	fact := Fact{Name: "limit", Value: "32Mi"}
	q := NewFactQuestion(fact, strings.Repeat("x", 2*MaxStateBytes))
	if len(q.State) != MaxStateBytes {
		t.Fatalf("state %d bytes", len(q.State))
	}
	if !strings.HasPrefix(q.State, "EXPECTED FACT\nlimit: 32Mi\n\nDIAGNOSIS\n") {
		t.Fatal(q.State[:60])
	}
	if !strings.Contains(q.Question, "(limit)") || len(q.Options) != 3 || q.Options[0] != Stated {
		t.Fatalf("question %q options %v", q.Question, q.Options)
	}
	if len(q.Question) > MaxQuestionBytes {
		t.Fatal("the question is too long for Jev")
	}
}
