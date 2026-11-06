package application

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// fakeClock advances only when slept on.
type fakeClock struct {
	mu  sync.Mutex
	now time.Time
	// onSleep runs before each sleep, to move the world along.
	onSleep func()
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeClock) Sleep(ctx context.Context, d time.Duration) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}
	if c.onSleep != nil {
		c.onSleep()
	}
	c.mu.Lock()
	c.now = c.now.Add(d)
	c.mu.Unlock()
	return nil
}

// fakeJudge answers every fact from a table of option by fact name.
type fakeJudge struct {
	answers map[string]string
	err     error
	asked   []domain.Question
}

func (j *fakeJudge) Choose(_ context.Context, q domain.Question) (domain.Choice, error) {
	j.asked = append(j.asked, q)
	if j.err != nil {
		return domain.Choice{}, j.err
	}
	for name, option := range j.answers {
		if len(q.State) > 14 && q.State[14:14+len(name)] == name {
			return domain.Choice{Option: option, Probability: 0.8, InputTokens: 100}, nil
		}
	}
	return domain.Choice{Option: domain.Stated, Probability: 0.9, InputTokens: 100}, nil
}

// fakeControlPlane holds incidents, a timeline, tasks and executions.
type fakeControlPlane struct {
	mu         sync.Mutex
	incidents  []domain.Incident
	timeline   domain.Timeline
	tasks      map[string]domain.Task
	executions map[string][]domain.Execution
	fail       map[string]error
	// submitted records each task asked for; onSubmit answers its id.
	submitted []submission
	onSubmit  func(submission) string
}

type submission struct{ bot, objective, key string }

func (cp *fakeControlPlane) SubmitTask(_ context.Context, bot, objective, key string) (string, error) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	if err := cp.fail["submit"]; err != nil {
		return "", err
	}
	s := submission{bot: bot, objective: objective, key: key}
	cp.submitted = append(cp.submitted, s)
	if cp.onSubmit != nil {
		return cp.onSubmit(s), nil
	}
	return "task-fix", nil
}

func (cp *fakeControlPlane) setTask(task domain.Task) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	if cp.tasks == nil {
		cp.tasks = map[string]domain.Task{}
	}
	cp.tasks[task.ID] = task
}

func (cp *fakeControlPlane) Incidents(context.Context, string) ([]domain.Incident, error) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	return append([]domain.Incident(nil), cp.incidents...), cp.fail["incidents"]
}

func (cp *fakeControlPlane) Incident(_ context.Context, id string) (domain.Incident, error) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	for _, i := range cp.incidents {
		if i.ID == id {
			return i, cp.fail["incident"]
		}
	}
	return domain.Incident{}, errors.New("no incident " + id)
}

func (cp *fakeControlPlane) Events(context.Context, string, int) (domain.Timeline, error) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	return cp.timeline, cp.fail["events"]
}

func (cp *fakeControlPlane) Task(_ context.Context, id string) (domain.Task, error) {
	if err := cp.fail["task"]; err != nil {
		return domain.Task{}, err
	}
	cp.mu.Lock()
	defer cp.mu.Unlock()
	task, ok := cp.tasks[id]
	if !ok {
		return domain.Task{}, errors.New("no task " + id)
	}
	return task, nil
}

func (cp *fakeControlPlane) Executions(_ context.Context, id string) ([]domain.Execution, error) {
	return cp.executions[id], cp.fail["executions"]
}

func (cp *fakeControlPlane) set(incident domain.Incident) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	for n, i := range cp.incidents {
		if i.ID == incident.ID {
			cp.incidents[n] = incident
			return
		}
	}
	cp.incidents = append(cp.incidents, incident)
}

func (cp *fakeControlPlane) event(kind, incidentID string, at time.Time) {
	cp.mu.Lock()
	defer cp.mu.Unlock()
	cp.timeline = append(cp.timeline, domain.Event{Type: kind, IncidentID: incidentID, CreatedAt: at})
}

// fakeLab records what was injected and cleared.
type fakeLab struct {
	expected map[string]domain.Expected
	injected []string
	cleared  []string
	clearErr error
	injErr   error
	onInject func(string)
	// writeNode names the scenarios only the write node handles.
	writeNode map[string]bool
}

func (l *fakeLab) NeedsWriteNode(scenario string) bool { return l.writeNode[scenario] }

func (l *fakeLab) Scenarios() ([]string, error) {
	var names []string
	for name := range l.expected {
		names = append(names, name)
	}
	return names, nil
}

func (l *fakeLab) Expected(scenario string) (domain.Expected, error) {
	e, ok := l.expected[scenario]
	if !ok {
		return domain.Expected{}, errors.New("no scenario " + scenario)
	}
	return e, nil
}

func (l *fakeLab) Inject(_ context.Context, scenario string) error {
	l.injected = append(l.injected, scenario)
	if l.onInject != nil {
		l.onInject(scenario)
	}
	return l.injErr
}

func (l *fakeLab) Clear(_ context.Context, scenario string) error {
	l.cleared = append(l.cleared, scenario)
	return l.clearErr
}

func groupKey(alert string) string {
	return `{}/{lab="true"}:{alertname="` + alert + `", namespace="shop"}`
}

func oomExpected() domain.Expected {
	return domain.Expected{Scenario: "oom", Alert: "LabPodOOMKilled", Facts: []domain.Fact{
		{Name: "object", Value: "Deployment `cart`"}, {Name: "limit", Value: "32Mi"}, {Name: "fix (not applied)", Value: "raise it"}}}
}

func driftExpected() domain.Expected {
	return domain.Expected{Scenario: "drift", Alert: "LabDeploymentDrift", Facts: []domain.Fact{
		{Name: "object", Value: "Deployment `web`"}, {Name: "evidence", Value: "differences: true"}, {Name: "rollback", Value: "3 replicas"}}}
}
