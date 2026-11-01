package results

import (
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func TestStoreKeepsAndLoads(t *testing.T) {
	store := Store{Dir: filepath.Join(t.TempDir(), "results")}
	run, err := store.Load()
	if err != nil || len(run.Results) != 0 {
		t.Fatalf("no directory is an empty run: %v %v", run, err)
	}
	result := domain.Result{Scenario: "oom", Alert: "A", IncidentID: "inc-1", Status: domain.IncidentDiagnosed, CostUSD: 0.01,
		InjectedAt: time.Date(2026, 11, 1, 0, 0, 0, 0, time.UTC), OpenToSettled: 30 * time.Second,
		Score: domain.NewScore([]domain.Verdict{{Fact: domain.Fact{Name: "object", Value: "cart"}, Choice: domain.Choice{Option: domain.Stated, Probability: 1, InputTokens: 10}}})}
	if err := store.Keep(result); err != nil {
		t.Fatal(err)
	}
	if err := store.Keep(domain.Result{Scenario: "drift"}); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(store.Dir, "notes.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	run, err = store.Load()
	if err != nil || len(run.Results) != 2 || run.Results[1].Scenario != "oom" {
		t.Fatalf("%+v %v", run, err)
	}
	loaded := run.Results[1]
	if loaded.CostUSD != 0.01 || loaded.OpenToSettled != 30*time.Second || loaded.Score.Outcome() != domain.Correct || !loaded.InjectedAt.Equal(result.InjectedAt) {
		t.Fatalf("%+v", loaded)
	}
	if err := os.WriteFile(filepath.Join(store.Dir, "bad.json"), []byte("{"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Load(); err == nil {
		t.Fatal("a broken file is an error")
	}
	if err := (Store{Dir: "/proc/nowhere"}).Keep(result); err == nil {
		t.Fatal("an unwritable directory is an error")
	}
	if _, err := (Store{Dir: filepath.Join(store.Dir, "notes.txt")}).Load(); err == nil {
		t.Fatal("a file is not a directory")
	}
}
