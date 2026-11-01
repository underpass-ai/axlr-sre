package application

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Settings bound a scenario's run.
type Settings struct {
	Tenant  string
	Bot     string
	Timeout time.Duration
	Poll    time.Duration
	// EventLimit is how many of the bot's newest events the judge reads
	// for the timeline.
	EventLimit int
}

// JudgeScenario injects one scenario, waits for its incident to leave
// Open, judges the diagnosis and clears the fault.
type JudgeScenario struct {
	Lab          Lab
	ControlPlane ControlPlane
	Clock        Clock
	Score        ScoreDiagnosis
	Cost         IncidentCost
	Settings     Settings
}

// ErrTimeout says the incident never left Open in time.
var ErrTimeout = errors.New("the incident did not leave Open in time")

// Run judges one scenario. When keep is set the fault is left in place
// (the soak scenario), unless it never settled; the result's Error
// carries what went wrong, and the fault is cleared then.
func (uc JudgeScenario) Run(ctx context.Context, scenario string, keep bool) (domain.Result, error) {
	result := domain.Result{Scenario: scenario}
	expected, err := uc.Lab.Expected(scenario)
	if err != nil {
		return result, err
	}
	result.Alert = expected.Alert
	before, err := uc.cycles(ctx, expected.Alert)
	if err != nil {
		return result, err
	}
	result.InjectedAt = uc.Clock.Now()
	if err := uc.Lab.Inject(ctx, scenario); err != nil {
		return result, fmt.Errorf("%s: inject: %w", scenario, err)
	}
	incident, err := uc.await(ctx, expected.Alert, before, &result)
	if err != nil {
		result.Error = err.Error()
	} else if err := uc.measure(ctx, expected, incident, &result); err != nil {
		result.Error = err.Error()
	}
	if !keep || result.Error != "" {
		if err := uc.Lab.Clear(ctx, scenario); err != nil {
			result.Error = joinError(result.Error, fmt.Sprintf("clear: %v", err))
		}
	}
	return result, nil
}

// Finish measures a kept (soaked) scenario again, then clears it.
func (uc JudgeScenario) Finish(ctx context.Context, result domain.Result) (domain.Result, error) {
	if result.IncidentID != "" {
		incident, err := uc.ControlPlane.Incident(ctx, result.IncidentID)
		if err != nil {
			return result, err
		}
		result.Repeats = incident.Repeats
		result.Status = incident.Status
		if err := uc.count(ctx, result.Alert, incident, &result); err != nil {
			return result, err
		}
	}
	if err := uc.Lab.Clear(ctx, result.Scenario); err != nil {
		result.Error = joinError(result.Error, fmt.Sprintf("clear: %v", err))
	}
	return result, nil
}

// cycles maps the alert's incidents to their cycle before the injection,
// so a reopening (same id, next cycle) counts as this scenario's.
func (uc JudgeScenario) cycles(ctx context.Context, alert string) (map[string]int, error) {
	incidents, err := uc.ControlPlane.Incidents(ctx, uc.Settings.Tenant)
	if err != nil {
		return nil, err
	}
	before := map[string]int{}
	for _, incident := range incidents {
		if incident.IsFor(alert) {
			before[incident.ID] = incident.Cycle
		}
	}
	return before, nil
}

func (uc JudgeScenario) await(ctx context.Context, alert string, before map[string]int, result *domain.Result) (domain.Incident, error) {
	deadline := result.InjectedAt.Add(uc.Settings.Timeout)
	for {
		incidents, err := uc.ControlPlane.Incidents(ctx, uc.Settings.Tenant)
		if err != nil {
			return domain.Incident{}, err
		}
		for _, incident := range incidents {
			if !incident.IsFor(alert) || !isNew(incident, before) {
				continue
			}
			if incident.Settled() {
				result.SettledAt = uc.Clock.Now()
				return incident, nil
			}
		}
		if !uc.Clock.Now().Before(deadline) {
			return domain.Incident{}, ErrTimeout
		}
		if err := uc.Clock.Sleep(ctx, uc.Settings.Poll); err != nil {
			return domain.Incident{}, err
		}
	}
}

func isNew(incident domain.Incident, before map[string]int) bool {
	cycle, seen := before[incident.ID]
	return !seen || incident.Cycle > cycle
}

func (uc JudgeScenario) measure(ctx context.Context, expected domain.Expected, incident domain.Incident, result *domain.Result) error {
	result.IncidentID = incident.ID
	result.Cycle = incident.Cycle
	result.Status = incident.Status
	result.Repeats = incident.Repeats
	result.Diagnosis = incident.Diagnosis
	if err := uc.count(ctx, expected.Alert, incident, result); err != nil {
		return err
	}
	cost, err := uc.Cost.Of(ctx, incident)
	if err != nil {
		return err
	}
	result.CostUSD = cost
	if incident.Status == domain.IncidentDiagnosed {
		score, err := uc.Score.Score(ctx, expected, incident.Diagnosis)
		if err != nil {
			return err
		}
		result.Score = score
	}
	return nil
}

// count reads the timeline and the other incidents: the trigger's
// answers, the cycle's Open → settled time, duplicates and side alerts.
func (uc JudgeScenario) count(ctx context.Context, alert string, incident domain.Incident, result *domain.Result) error {
	timeline, err := uc.ControlPlane.Events(ctx, uc.Settings.Bot, uc.Settings.EventLimit)
	if err != nil {
		return err
	}
	own := timeline.Of(incident.ID, result.InjectedAt)
	result.Opened = own.Count(domain.EventIncidentOpened)
	result.Reopened = own.Count(domain.EventIncidentReopened)
	result.Repeated = own.Count(domain.EventIncidentRepeated)
	result.OpenToSettled = own.OpenToSettled()
	incidents, err := uc.ControlPlane.Incidents(ctx, uc.Settings.Tenant)
	if err != nil {
		return err
	}
	result.Duplicates, result.OtherAlerts = 0, nil
	for _, other := range incidents {
		if other.ID == incident.ID || other.OpenedAt.Before(result.InjectedAt) {
			continue
		}
		if other.IsFor(alert) {
			result.Duplicates++
		} else {
			result.OtherAlerts = append(result.OtherAlerts, other.Alert())
		}
	}
	return nil
}

func joinError(first, second string) string {
	if first == "" {
		return second
	}
	return first + "; " + second
}
