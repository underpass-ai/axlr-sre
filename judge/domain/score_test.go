package domain

import (
	"math"
	"testing"
)

func verdict(name, option string) Verdict {
	return Verdict{Fact: Fact{Name: name, Value: "v"}, Choice: Choice{Option: option, Probability: 0.9, InputTokens: 1000}}
}

func TestScoreOutcomes(t *testing.T) {
	cases := []struct {
		name     string
		verdicts []Verdict
		outcome  Outcome
		missing  int
	}{
		{"all stated", []Verdict{verdict("a", Stated), verdict("b", Stated), verdict("fix", Missing)}, Correct, 0},
		{"some stated", []Verdict{verdict("a", Stated), verdict("b", Missing)}, Partly, 1},
		{"contradicted", []Verdict{verdict("a", Stated), verdict("b", Contradicted)}, Wrong, 1},
		{"none stated", []Verdict{verdict("a", Missing)}, Wrong, 1},
		{"no facts", nil, Wrong, 0},
		{"only the fix", []Verdict{verdict("fix (not applied)", Stated)}, Wrong, 0},
	}
	for _, c := range cases {
		score := NewScore(c.verdicts)
		if got := score.Outcome(); got != c.outcome {
			t.Errorf("%s: outcome %s, want %s", c.name, got, c.outcome)
		}
		if got := len(score.MissingFacts()); got != c.missing {
			t.Errorf("%s: %d missing, want %d", c.name, got, c.missing)
		}
	}
}

func TestScoreCountsFixAndCost(t *testing.T) {
	score := NewScore([]Verdict{verdict("a", Stated), verdict("fix", Stated)})
	if !score.FixStated() || score.CauseTotal() != 1 || score.CauseStated() != 1 {
		t.Fatalf("%+v", score)
	}
	if want := 2000 * JevInputPricePerToken; math.Abs(score.JevCostUSD()-want) > 1e-12 {
		t.Fatalf("cost %v want %v", score.JevCostUSD(), want)
	}
	if NewScore([]Verdict{verdict("fix", Missing)}).FixStated() {
		t.Fatal("a missing fix is not stated")
	}
	marks := []string{verdict("a", Stated).Mark(), verdict("a", Contradicted).Mark(), verdict("a", Missing).Mark()}
	if marks[0] != "yes" || marks[1] != "no" || marks[2] != "missing" {
		t.Fatal(marks)
	}
}
