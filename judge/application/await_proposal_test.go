package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// fixed is the drift incident once triage diagnosed it, with the fix task
// the control plane launched.
func fixed(status string) domain.Incident {
	incident := domain.Incident{ID: "inc-7", GroupKey: `{}:{alertname="LabDeploymentDrift"}`, Status: status, Cycle: 2, CostUSD: 0.004,
		Diagnosis: "CAUSE: drift-lab/web has 3 replicas, desired 2", TaskIDs: []string{"t1", "task-fix"},
		Tasks: []domain.IncidentTask{{TaskID: "t1", BotID: "triage", Phase: domain.PhaseTriage, Cycle: 2}, {TaskID: "task-fix", BotID: "fix", Phase: domain.PhaseFix, Cycle: 2}}}
	if status == domain.IncidentProposed {
		incident.Proposal, incident.CostUSD = "PROPOSAL: apply desired", 0.007
	}
	return incident
}

// fixWorld keeps the incident Diagnosed for the given polls, then moves
// it to the status; the fix task was created at t0.
func fixWorld(polls int, status string) (*fakeControlPlane, *fakeClock, AwaitProposal) {
	cp := &fakeControlPlane{executions: map[string][]domain.Execution{"task-fix": {{CostUSD: 0.001}, {CostUSD: 0.002, Answer: "PROPOSAL: apply desired"}}}}
	cp.set(fixed(domain.IncidentDiagnosed))
	taskStatus := domain.TaskCompleted
	if status != domain.IncidentProposed {
		taskStatus = domain.TaskFailed
	}
	cp.setTask(domain.Task{ID: "task-fix", Status: "Running", CreatedAt: t0})
	clock := &fakeClock{now: t0}
	seen := 0
	clock.onSleep = func() {
		seen++
		if seen >= polls {
			cp.set(fixed(status))
			cp.setTask(domain.Task{ID: "task-fix", Status: taskStatus, CreatedAt: t0})
		}
	}
	return cp, clock, AwaitProposal{ControlPlane: cp, Clock: clock, Timeout: time.Minute, Poll: 5 * time.Second}
}

func TestAwaitProposalReadsWhatTheFixPhaseAnswered(t *testing.T) {
	_, _, uc := fixWorld(3, domain.IncidentProposed)
	proposal, final, err := uc.Run(context.Background(), fixed(domain.IncidentDiagnosed))
	if err != nil {
		t.Fatal(err)
	}
	if !proposal.Completed() || proposal.Answer != "PROPOSAL: apply desired" || proposal.TaskID != "task-fix" || final.Status != domain.IncidentProposed {
		t.Fatalf("%+v %+v", proposal, final)
	}
	if proposal.CostUSD < 0.0029 || proposal.CostUSD > 0.0031 || proposal.Took() != 15*time.Second {
		t.Fatalf("cost %v, took %v", proposal.CostUSD, proposal.Took())
	}
	// An incident already Proposed is read at once.
	cp, clock, uc := fixWorld(1, domain.IncidentProposed)
	cp.setTask(domain.Task{ID: "task-fix", Status: domain.TaskCompleted, CreatedAt: t0})
	clock.onSleep = func() { t.Fatal("slept on a Proposed incident") }
	if proposal, _, err := uc.Run(context.Background(), fixed(domain.IncidentProposed)); err != nil || !proposal.Completed() {
		t.Fatalf("%+v %v", proposal, err)
	}
}

func TestAwaitProposalReportsAFixPhaseThatFailsNeverEndsOrNeverStarts(t *testing.T) {
	_, _, uc := fixWorld(1, domain.IncidentEscalated)
	proposal, final, err := uc.Run(context.Background(), fixed(domain.IncidentDiagnosed))
	if err != nil || proposal.Status != domain.TaskFailed || proposal.Completed() || final.Status != domain.IncidentEscalated {
		t.Fatalf("an escalated fix ends without a proposal: %+v %v", proposal, err)
	}

	_, _, uc = fixWorld(100, domain.IncidentProposed)
	proposal, _, err = uc.Run(context.Background(), fixed(domain.IncidentDiagnosed))
	if !errors.Is(err, ErrProposalTimeout) || proposal.TaskID != "task-fix" || proposal.Completed() {
		t.Fatalf("%+v %v", proposal, err)
	}

	// No teammate takes the fix phase: Diagnosed with no fix task.
	cp, _, uc := fixWorld(100, domain.IncidentProposed)
	alone := fixed(domain.IncidentDiagnosed)
	alone.Tasks, alone.TaskIDs = alone.Tasks[:1], alone.TaskIDs[:1]
	cp.set(alone)
	if _, _, err := uc.Run(context.Background(), alone); !errors.Is(err, ErrNoFixPhase) || !strings.Contains(err.Error(), `"phase": "fix"`) {
		t.Fatalf("%v", err)
	}
	resolved := alone
	resolved.Status = domain.IncidentResolved
	if _, _, err := uc.Run(context.Background(), resolved); !errors.Is(err, ErrNoFixPhase) {
		t.Fatalf("resolved without a fix task: %v", err)
	}

	for _, broken := range []string{"incident", "task", "executions"} {
		cp, _, uc := fixWorld(1, domain.IncidentProposed)
		cp.fail = map[string]error{broken: errors.New(broken + " down")}
		if _, _, err := uc.Run(context.Background(), fixed(domain.IncidentDiagnosed)); err == nil || !strings.Contains(err.Error(), broken+" down") {
			t.Fatalf("%s: %v", broken, err)
		}
	}

	_, clock, uc := fixWorld(100, domain.IncidentProposed)
	ctx, cancel := context.WithCancel(context.Background())
	clock.onSleep = cancel
	if _, _, err := uc.Run(ctx, fixed(domain.IncidentDiagnosed)); !errors.Is(err, context.Canceled) {
		t.Fatalf("%v", err)
	}
}

// writeWorld is newWorld with oom as a write-node scenario: the control
// plane launches the fix task when triage diagnoses, and the incident is
// in the status given (Proposed with the answer) once it ends.
func writeWorld(answer, status string) *world {
	w := newWorld()
	w.lab.writeNode = map[string]bool{"oom": true}
	taskStatus := domain.TaskCompleted
	if status != domain.IncidentProposed {
		taskStatus = domain.TaskFailed
	}
	w.cp.setTask(domain.Task{ID: "task-fix", Status: taskStatus, CreatedAt: t0})
	w.cp.executions["task-fix"] = []domain.Execution{{CostUSD: 0.003, Answer: answer}}
	w.uc.Propose = AwaitProposal{ControlPlane: w.cp, Clock: w.clock, Timeout: time.Minute, Poll: 5 * time.Second}
	w.afterSettle = func(incident domain.Incident) domain.Incident {
		incident.Status, incident.Proposal, incident.CostUSD = status, answer, incident.CostUSD+0.003
		return incident
	}
	return w
}

func TestJudgeScenarioScoresTheFixPhasesProposalOnTheWriteNode(t *testing.T) {
	w := writeWorld("PROPOSAL: raise the limit; ROLLBACK: 32Mi", domain.IncidentProposed)
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if !result.WriteNode || result.Proposal == nil || result.Proposal.Answer != "PROPOSAL: raise the limit; ROLLBACK: 32Mi" || result.Status != domain.IncidentProposed || result.Outcome() != domain.Correct {
		t.Fatalf("%+v", result)
	}
	// triage 0.005 and fix 0.003 from the incident, the child c1 0.002.
	if result.CostUSD < 0.0099 || result.CostUSD > 0.0101 {
		t.Fatalf("the incident's cost holds the fix task's: %v", result.CostUSD)
	}
	if !strings.Contains(w.judge.asked[0].State, "PROPOSAL: raise the limit") {
		t.Fatalf("Jev reads the proposal, not the diagnosis: %q", w.judge.asked[0].State)
	}
}

func TestJudgeScenarioFailsAWriteNodeScenarioWithoutAProposal(t *testing.T) {
	w := writeWorld("", domain.IncidentEscalated)
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(result.Error, "fix task task-fix ended Failed without a proposal (incident Escalated)") || result.Outcome() != domain.Wrong || len(w.judge.asked) != 0 {
		t.Fatalf("%+v", result)
	}

	w = writeWorld("x", domain.IncidentProposed)
	w.withoutFixTask = true
	w.opens("inc-2", 1, domain.IncidentDiagnosed)
	result, _ = w.uc.Run(context.Background(), "oom", false)
	if !strings.Contains(result.Error, "fix: the control plane launched no fix task") || result.Outcome() != domain.Wrong {
		t.Fatalf("%+v", result)
	}

	w = writeWorld("x", domain.IncidentProposed)
	w.opens("inc-3", 1, domain.IncidentEscalated)
	result, _ = w.uc.Run(context.Background(), "oom", false)
	if !result.WriteNode || result.Proposal != nil || result.Outcome() != domain.Wrong {
		t.Fatalf("an escalated incident has no fix phase: %+v", result)
	}
}

// A read-node scenario is judged on its diagnosis, even when the fix
// phase already made the incident Proposed.
func TestJudgeScenarioScoresTheDiagnosisOfAProposedIncident(t *testing.T) {
	w := newWorld()
	w.opens("inc-1", 1, domain.IncidentProposed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if result.Proposal != nil || result.Status != domain.IncidentProposed || result.Outcome() != domain.Correct || !strings.Contains(w.judge.asked[0].State, "OOMKilled") {
		t.Fatalf("%+v", result)
	}
}
