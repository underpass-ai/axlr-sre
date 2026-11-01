package application

import (
	"context"
	"fmt"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Soak keeps one scenario firing for a while after its diagnosis, to
// count the resends that must stay one incident. It is cleared when the
// other scenarios are done and its time is up.
type Soak struct {
	Scenario string
	For      time.Duration
}

// RunScenarios judges the scenarios one after another and hands each
// result over as it lands, so a run that breaks can resume from its
// files.
type RunScenarios struct {
	Scenario JudgeScenario
	Clock    Clock
	Soak     Soak
	// Keep stores a result; a nil Keep stores nothing.
	Keep func(domain.Result) error
	// Say reports progress in a line; nil says nothing.
	Say func(string)
}

// Run judges the scenarios in the order given.
func (uc RunScenarios) Run(ctx context.Context, scenarios []string) (domain.Run, error) {
	var results []domain.Result
	var soaked *domain.Result
	var soakedAt time.Time
	for _, scenario := range scenarios {
		keep := scenario == uc.Soak.Scenario && uc.Soak.For > 0
		uc.say("%s: injecting", scenario)
		result, err := uc.Scenario.Run(ctx, scenario, keep)
		if err != nil {
			return domain.NewRun(results), err
		}
		uc.say("%s: %s", scenario, summary(result))
		if keep && result.Error == "" {
			soaked, soakedAt = &result, uc.Clock.Now()
			continue
		}
		if err := uc.keep(result); err != nil {
			return domain.NewRun(results), err
		}
		results = append(results, result)
	}
	if soaked != nil {
		if wait := uc.Soak.For - uc.Clock.Now().Sub(soakedAt); wait > 0 {
			uc.say("%s: holding %s more for the resends", soaked.Scenario, wait.Round(time.Second))
			if err := uc.Clock.Sleep(ctx, wait); err != nil {
				return domain.NewRun(results), err
			}
		}
		result, err := uc.Scenario.Finish(ctx, *soaked)
		if err != nil {
			return domain.NewRun(results), err
		}
		uc.say("%s: %d resends, %d duplicates after the soak", result.Scenario, result.Repeats, result.Duplicates)
		if err := uc.keep(result); err != nil {
			return domain.NewRun(results), err
		}
		results = append(results, result)
	}
	return domain.NewRun(results), nil
}

func (uc RunScenarios) keep(result domain.Result) error {
	if uc.Keep == nil {
		return nil
	}
	return uc.Keep(result)
}

func (uc RunScenarios) say(format string, args ...any) {
	if uc.Say != nil {
		uc.Say(fmt.Sprintf(format, args...))
	}
}

func summary(result domain.Result) string {
	if result.Error != "" {
		return "no verdict: " + result.Error
	}
	return fmt.Sprintf("%s after %s (Open → %s %s), cause %s (%d/%d facts), $%.4f",
		result.Status, result.InjectToSettled().Round(time.Second), result.Status,
		result.OpenToSettled.Round(time.Second), result.Outcome(),
		result.Score.CauseStated(), result.Score.CauseTotal(), result.CostUSD)
}
