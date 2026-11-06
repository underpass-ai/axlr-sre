package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func diagnosedDrift() domain.Incident {
	return domain.Incident{ID: "inc-7", GroupKey: `{}:{alertname="LabDeploymentDrift"}`, Status: domain.IncidentDiagnosed, Cycle: 2,
		Diagnosis: "CAUSE: drift-lab/web has 3 replicas, desired 2", TaskIDs: []string{"t1"}}
}

// fixWorld runs a fix task that is Running for the given polls, then
// ends with the status.
func fixWorld(polls int, status string) (*fakeControlPlane, *fakeClock, ProposeFix) {
	cp := &fakeControlPlane{executions: map[string][]domain.Execution{"task-fix": {{CostUSD: 0.001}, {CostUSD: 0.002, Answer: "PROPOSAL: apply desired"}}}}
	cp.setTask(domain.Task{ID: "task-fix", Status: "Pending"})
	clock := &fakeClock{now: t0}
	seen := 0
	clock.onSleep = func() {
		seen++
		if seen >= polls {
			cp.setTask(domain.Task{ID: "task-fix", Status: status})
		} else {
			cp.setTask(domain.Task{ID: "task-fix", Status: "Running"})
		}
	}
	return cp, clock, ProposeFix{ControlPlane: cp, Clock: clock, Bot: "fix", Timeout: time.Minute, Poll: 5 * time.Second}
}

func TestProposeFixSubmitsTheDiagnosisAndReadsTheAnswer(t *testing.T) {
	cp, _, uc := fixWorld(3, domain.TaskCompleted)
	proposal, err := uc.Run(context.Background(), diagnosedDrift())
	if err != nil {
		t.Fatal(err)
	}
	if len(cp.submitted) != 1 {
		t.Fatalf("%+v", cp.submitted)
	}
	s := cp.submitted[0]
	if s.bot != "fix" || s.key != "sre-judge:inc-7:2:fix" || !strings.Contains(s.objective, "inc-7 (alert LabDeploymentDrift)") ||
		!strings.Contains(s.objective, "3 replicas, desired 2") || !strings.Contains(s.objective, "apply nothing") {
		t.Fatalf("%+v", s)
	}
	if !proposal.Completed() || proposal.Answer != "PROPOSAL: apply desired" || proposal.TaskID != "task-fix" {
		t.Fatalf("%+v", proposal)
	}
	if proposal.CostUSD < 0.0029 || proposal.CostUSD > 0.0031 || proposal.Took() != 15*time.Second {
		t.Fatalf("cost %v, took %v", proposal.CostUSD, proposal.Took())
	}
}

func TestProposeFixReportsATaskThatNeverEndsOrBreaks(t *testing.T) {
	_, _, uc := fixWorld(100, domain.TaskCompleted)
	proposal, err := uc.Run(context.Background(), diagnosedDrift())
	if !errors.Is(err, ErrProposalTimeout) || proposal.Status != "Running" || proposal.Completed() {
		t.Fatalf("%+v %v", proposal, err)
	}

	cp, _, uc := fixWorld(1, domain.TaskExpired)
	cp.executions = map[string][]domain.Execution{"task-fix": {{CostUSD: 0.001}}}
	proposal, err = uc.Run(context.Background(), diagnosedDrift())
	if err != nil || proposal.Status != domain.TaskExpired || proposal.Completed() {
		t.Fatalf("an expired task ends without a proposal: %+v %v", proposal, err)
	}

	for _, broken := range []string{"submit", "task", "executions"} {
		cp, _, uc := fixWorld(1, domain.TaskCompleted)
		cp.fail = map[string]error{broken: errors.New(broken + " down")}
		if _, err := uc.Run(context.Background(), diagnosedDrift()); err == nil || !strings.Contains(err.Error(), broken+" down") {
			t.Fatalf("%s: %v", broken, err)
		}
	}

	_, clock, uc := fixWorld(100, domain.TaskCompleted)
	ctx, cancel := context.WithCancel(context.Background())
	clock.onSleep = cancel
	if _, err := uc.Run(ctx, diagnosedDrift()); !errors.Is(err, context.Canceled) {
		t.Fatalf("%v", err)
	}
}

// writeWorld is newWorld with oom as a write-node scenario whose fix
// task completes with the answer.
func writeWorld(answer, status string) *world {
	w := newWorld()
	w.lab.writeNode = map[string]bool{"oom": true}
	w.cp.setTask(domain.Task{ID: "task-fix", Status: status})
	w.cp.executions["task-fix"] = []domain.Execution{{CostUSD: 0.003, Answer: answer}}
	w.uc.Propose = ProposeFix{ControlPlane: w.cp, Clock: w.clock, Bot: "fix", Timeout: time.Minute, Poll: 5 * time.Second}
	return w
}

func TestJudgeScenarioScoresFixsProposalOnTheWriteNode(t *testing.T) {
	w := writeWorld("PROPOSAL: raise the limit; ROLLBACK: 32Mi", domain.TaskCompleted)
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if !result.WriteNode || result.Proposal == nil || result.Proposal.Answer != "PROPOSAL: raise the limit; ROLLBACK: 32Mi" || result.Outcome() != domain.Correct {
		t.Fatalf("%+v", result)
	}
	if result.CostUSD < 0.0099 || result.CostUSD > 0.0101 {
		t.Fatalf("the proposal's cost is added: %v", result.CostUSD)
	}
	if !strings.Contains(w.judge.asked[0].State, "PROPOSAL: raise the limit") {
		t.Fatalf("Jev reads the proposal, not the diagnosis: %q", w.judge.asked[0].State)
	}
}

func TestJudgeScenarioFailsAWriteNodeScenarioWithoutAProposal(t *testing.T) {
	w := writeWorld("", domain.TaskFailed)
	w.opens("inc-1", 1, domain.IncidentDiagnosed)
	result, err := w.uc.Run(context.Background(), "oom", false)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(result.Error, "fix task task-fix ended Failed") || result.Outcome() != domain.Wrong || len(w.judge.asked) != 0 {
		t.Fatalf("%+v", result)
	}

	w = writeWorld("x", domain.TaskCompleted)
	w.cp.fail = map[string]error{"submit": errors.New("socket gone")}
	w.opens("inc-2", 1, domain.IncidentDiagnosed)
	result, _ = w.uc.Run(context.Background(), "oom", false)
	if !strings.Contains(result.Error, "fix: submit to fix: socket gone") || result.Outcome() != domain.Wrong {
		t.Fatalf("%+v", result)
	}

	w = writeWorld("x", domain.TaskCompleted)
	w.opens("inc-3", 1, domain.IncidentEscalated)
	result, _ = w.uc.Run(context.Background(), "oom", false)
	if !result.WriteNode || result.Proposal != nil || len(w.cp.submitted) != 0 || result.Outcome() != domain.Wrong {
		t.Fatalf("an escalated incident gets no fix task: %+v", result)
	}
}
