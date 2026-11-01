package application

import (
	"context"
	"fmt"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// ScoreDiagnosis asks Jev, fact by fact, whether a diagnosis states what
// the scenario expects.
type ScoreDiagnosis struct {
	Judge Judge
}

// Score judges every fact of the expectation against the diagnosis.
func (uc ScoreDiagnosis) Score(ctx context.Context, expected domain.Expected, diagnosis string) (domain.Score, error) {
	var verdicts []domain.Verdict
	for _, fact := range expected.Facts {
		choice, err := uc.Judge.Choose(ctx, domain.NewFactQuestion(fact, diagnosis))
		if err != nil {
			return domain.Score{}, fmt.Errorf("%s, fact %q: %w", expected.Scenario, fact.Name, err)
		}
		verdicts = append(verdicts, domain.Verdict{Fact: fact, Choice: choice})
	}
	return domain.NewScore(verdicts), nil
}
