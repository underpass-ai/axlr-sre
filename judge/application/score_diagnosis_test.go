package application

import (
	"context"
	"errors"
	"testing"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func TestScoreDiagnosisAsksOneQuestionPerFact(t *testing.T) {
	judge := &fakeJudge{answers: map[string]string{"limit": domain.Missing}}
	score, err := ScoreDiagnosis{Judge: judge}.Score(context.Background(), oomExpected(), "cart is OOMKilled")
	if err != nil {
		t.Fatal(err)
	}
	if len(judge.asked) != 3 || score.Outcome() != domain.Partly || score.CauseStated() != 1 {
		t.Fatalf("asked %d, outcome %s, stated %d", len(judge.asked), score.Outcome(), score.CauseStated())
	}
	if !score.FixStated() {
		t.Fatal("the fix was stated")
	}
}

func TestScoreDiagnosisNamesTheFactThatFailed(t *testing.T) {
	judge := &fakeJudge{err: errors.New("TypeSafe did not answer")}
	_, err := ScoreDiagnosis{Judge: judge}.Score(context.Background(), oomExpected(), "x")
	if err == nil || err.Error() != `oom, fact "object": TypeSafe did not answer` {
		t.Fatalf("%v", err)
	}
}
