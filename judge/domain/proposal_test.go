package domain

import (
	"strings"
	"testing"
	"time"
)

func TestProposalCompletesOnlyWithAnAnswer(t *testing.T) {
	done := Proposal{Status: TaskCompleted, Answer: "PROPOSAL: apply", SubmittedAt: time.Unix(10, 0), EndedAt: time.Unix(52, 0)}
	if !done.Completed() || done.Took() != 42*time.Second {
		t.Fatalf("%+v", done)
	}
	for _, p := range []Proposal{{Status: TaskCompleted, Answer: "  "}, {Status: TaskFailed, Answer: "x"}, {}} {
		if p.Completed() || p.Took() != 0 {
			t.Fatalf("%+v", p)
		}
	}
	for status, ended := range map[string]bool{TaskCompleted: true, TaskFailed: true, TaskExpired: true, TaskCancelled: true, "Running": false, "WaitingApproval": false} {
		if TaskEnded(status) != ended {
			t.Fatalf("%s: %v", status, !ended)
		}
	}
}

func TestFixObjectiveCarriesTheIncidentAndTheDiagnosis(t *testing.T) {
	objective := FixObjective(Incident{ID: "inc-7", GroupKey: `{}:{alertname="LabDeploymentDrift"}`, Diagnosis: "  CAUSE: drift  "})
	for _, want := range []string{"Incident inc-7 (alert LabDeploymentDrift)", "\n\nCAUSE: drift\n\n", "apply nothing"} {
		if !strings.Contains(objective, want) {
			t.Fatalf("missing %q in %q", want, objective)
		}
	}
}

func TestAWriteNodeResultNeedsAProposal(t *testing.T) {
	r := result("drift", Correct, 0.01, 0)
	r.WriteNode = true
	if r.Outcome() != Wrong {
		t.Fatal("no proposal is wrong")
	}
	r.Proposal = &Proposal{Status: TaskExpired}
	if r.Outcome() != Wrong {
		t.Fatal("a fix task that ended without an answer is wrong")
	}
	r.Proposal = &Proposal{Status: TaskCompleted, Answer: "PROPOSAL"}
	if r.Outcome() != Correct {
		t.Fatal(r.Outcome())
	}
}

func TestByNodeJudgesEachNodeAgainstItsGoals(t *testing.T) {
	var results []Result
	for i := 0; i < 9; i++ {
		outcome := Correct
		if i == 0 {
			outcome = Partly
		}
		results = append(results, result(string(rune('a'+i)), outcome, 0.01, 0))
	}
	drift := result("drift", Correct, 0.01, 0)
	drift.WriteNode, drift.Proposal = true, &Proposal{Status: TaskCompleted, Answer: "PROPOSAL"}
	run := NewRun(append(results, drift))

	parts := run.ByNode()
	if len(parts) != 2 || parts[0].Goals.Name != ReadNode.Name || len(parts[0].Run.Results) != 9 ||
		parts[1].Goals.Name != WriteNode.Name || len(parts[1].Run.Results) != 1 {
		t.Fatalf("%+v", parts)
	}
	if !run.PassesByNode() {
		t.Fatal("8/9 on the read node and 1/1 on the write node pass")
	}
	drift.Proposal = nil
	if NewRun(append(results, drift)).PassesByNode() {
		t.Fatal("the write node's part fails without a proposal")
	}
	if len(NewRun(results).ByNode()) != 1 || len(NewRun(nil).ByNode()) != 0 || !NewRun(nil).PassesByNode() {
		t.Fatal("a part without scenarios is left out")
	}
	if WriteNode.Judged != "proposals" || ReadNode.Judged != "causes" || Phase0.Judged != "causes" {
		t.Fatal("what Jev reads")
	}
}
