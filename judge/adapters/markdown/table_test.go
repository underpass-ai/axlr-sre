package markdown

import (
	"strings"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func TestRenderShowsTotalsRowsAndFacts(t *testing.T) {
	choice := func(option string) domain.Choice {
		return domain.Choice{Option: option, Probability: 0.9, InputTokens: 100}
	}
	results := []domain.Result{
		{Scenario: "oom", Alert: "LabPodOOMKilled", IncidentID: "inc-1", Status: domain.IncidentDiagnosed, Opened: 1, Repeated: 2,
			InjectedAt: time.Unix(0, 0), SettledAt: time.Unix(160, 0), OpenToSettled: 40 * time.Second, CostUSD: 0.008, Repeats: 2,
			OtherAlerts: []string{"LabPodCrashLooping"},
			Score: domain.NewScore([]domain.Verdict{
				{Fact: domain.Fact{Name: "object", Value: "cart"}, Choice: choice(domain.Stated)},
				{Fact: domain.Fact{Name: "limit", Value: "32Mi"}, Choice: choice(domain.Missing)},
				{Fact: domain.Fact{Name: "fix (not applied)", Value: "raise"}, Choice: choice(domain.Stated)}})},
		{Scenario: "pvc-full", Alert: "LabVolumeFull", Error: "the incident did not leave Open in time", InjectedAt: time.Unix(0, 0)},
		{Scenario: "drift", Alert: "LabDeploymentDrift", IncidentID: "inc-2", Status: domain.IncidentDiagnosed, Reopened: 1, Duplicates: 1,
			InjectedAt: time.Unix(0, 0), SettledAt: time.Unix(100, 0), OpenToSettled: 30 * time.Second, CostUSD: 0.004,
			Score: domain.NewScore([]domain.Verdict{{Fact: domain.Fact{Name: "object", Value: "web"}, Choice: choice(domain.Stated)}})},
	}
	out := Render(domain.NewRun(results), domain.Phase0)
	for _, want := range []string{
		"| scenarios that fired their alert and opened one incident | 2 / 3 | 10 / 10 |",
		"| causes Jev found stated in full (partly) | 1 / 3 (1) | 8 / 10 |",
		"| duplicate incidents | 1 | 0 |",
		"| Open → Diagnosed | 30 s–40 s | < 3m0s |",
		"| cost of the whole run, children included | $0.012 | < $1 |",
		"| passes | false | |",
		"| oom | `LabPodOOMKilled` | opened, repeated ×2 | 160 s | 40 s | $0.0080 | 2 | 0 | LabPodCrashLooping | 1 / 2 (not limit) | yes | **partly** |",
		"| pvc-full | `LabVolumeFull` | — | — | — | $0.0000 | 0 | 0 | — | — | — | **wrong: the incident did not leave Open in time** |",
		"| drift | `LabDeploymentDrift` | reopened | 100 s | 30 s | $0.0040 | 0 | 1 | — | 1 / 1 | — | **correct** |",
		"### oom\n\n| Fact | Expected | Jev | p |",
		"| limit | 32Mi | missing | 0.90 |",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in\n%s", want, out)
		}
	}
	if strings.Contains(out, "### pvc-full") {
		t.Fatal("no facts for a scenario without a verdict")
	}
	if !strings.Contains(Render(domain.NewRun(nil), domain.Phase0), "| Open → Diagnosed | — |") {
		t.Fatal("an empty run has no spread")
	}
}

func TestRenderByNodeJudgesEachNodeAgainstItsGoals(t *testing.T) {
	stated := domain.Choice{Option: domain.Stated, Probability: 0.9}
	read := domain.Result{Scenario: "oom", Alert: "LabPodOOMKilled", IncidentID: "inc-1", Status: domain.IncidentDiagnosed,
		Score: domain.NewScore([]domain.Verdict{{Fact: domain.Fact{Name: "object", Value: "cart"}, Choice: stated}})}
	write := domain.Result{Scenario: "drift", Alert: "LabDeploymentDrift", IncidentID: "inc-2", Status: domain.IncidentDiagnosed, WriteNode: true,
		Proposal: &domain.Proposal{TaskID: "task-fix", Status: domain.TaskCompleted, Answer: "apply desired", CostUSD: 0.004,
			SubmittedAt: time.Unix(0, 0), EndedAt: time.Unix(42, 0)},
		Score: domain.NewScore([]domain.Verdict{{Fact: domain.Fact{Name: "rollback", Value: "3 replicas"}, Choice: stated}})}

	both := RenderByNode(domain.NewRun([]domain.Result{read, write}))
	for _, want := range []string{
		"## Read node\n\n| Measure | Result | Read node goal |",
		"| causes Jev found stated in full (partly) | 1 / 1 (0) | 8 / 9 |",
		"## Write node\n\n| Measure | Result | Write node goal |",
		"| proposals Jev found stated in full (partly) | 1 / 1 (0) | 1 / 1 |",
		"### drift: fix's proposal (task task-fix, Completed in 42 s, $0.0040)",
	} {
		if !strings.Contains(both, want) {
			t.Errorf("missing %q in\n%s", want, both)
		}
	}
	only := RenderByNode(domain.NewRun([]domain.Result{write}))
	if strings.HasPrefix(only, "## ") || strings.Contains(only, "\n## ") || !strings.Contains(only, "| Write node goal |") || !strings.Contains(only, "| passes | true | |") {
		t.Fatalf("a run of one node has its table alone:\n%s", only)
	}
}
