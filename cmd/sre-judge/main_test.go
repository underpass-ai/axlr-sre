package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/underpass-ai/axlr-sre/judge/adapters/jev"
	"github.com/underpass-ai/axlr-sre/judge/adapters/shell"
)

// world is a fake control plane, a fake TypeSafe and a scenarios
// directory whose scripts touch files.
type world struct {
	cp, jev    *httptest.Server
	scenarios  string
	out        string
	mu         sync.Mutex
	injections int
}

func newWorld(t *testing.T) *world {
	t.Helper()
	w := &world{}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/incidents", func(rw http.ResponseWriter, r *http.Request) {
		w.mu.Lock()
		defer w.mu.Unlock()
		var incidents []map[string]any
		if w.injections > 0 {
			incidents = append(incidents, map[string]any{"id": "inc-1", "groupKey": `{}/{}:{alertname="LabPodOOMKilled"}`, "status": "Diagnosed", "cycle": 1,
				"repeats": 3, "costUsd": 0.004, "diagnosis": "cart is OOMKilled under 32Mi", "taskIds": []string{"t1"}, "openedAt": "2026-11-01T10:00:00Z"})
		}
		_ = json.NewEncoder(rw).Encode(incidents)
	})
	mux.HandleFunc("GET /v1/incidents/{id}", func(rw http.ResponseWriter, r *http.Request) {
		_, _ = rw.Write([]byte(`{"id":"inc-1","status":"Diagnosed","diagnosis":"cart is OOMKilled under 32Mi","repeats":3}`))
	})
	mux.HandleFunc("GET /v1/bots/{id}/events", func(rw http.ResponseWriter, r *http.Request) {
		_, _ = rw.Write([]byte(`[{"type":"incident.opened","incidentId":"inc-1","createdAt":"2100-01-01T00:00:00Z"},{"type":"incident.diagnosed","incidentId":"inc-1","createdAt":"2100-01-01T00:00:40Z"}]`))
	})
	mux.HandleFunc("GET /v1/tasks/{id}", func(rw http.ResponseWriter, r *http.Request) {
		_, _ = rw.Write([]byte(`{"id":"t1","children":[]}`))
	})
	w.cp = httptest.NewServer(mux)
	w.jev = httptest.NewServer(http.HandlerFunc(func(rw http.ResponseWriter, r *http.Request) {
		var body struct {
			State string `json:"state"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		choice := "stated"
		if strings.HasPrefix(body.State, "EXPECTED FACT\nlimit") {
			choice = "missing"
		}
		_ = json.NewEncoder(rw).Encode(map[string]any{"model": jev.DefaultModel, "answers": map[string]any{"q": map[string]any{
			"type": "choice", "choice": choice, "probabilities": map[string]any{"stated": 0.7, "missing": 0.2, "contradicted": 0.1}, "confidence": 0.9}},
			"usage": map[string]any{"input_tokens": 100}})
	}))
	t.Cleanup(w.cp.Close)
	t.Cleanup(w.jev.Close)
	w.scenarios = filepath.Join(t.TempDir(), "scenarios")
	w.out = filepath.Join(t.TempDir(), "results")
	marks := filepath.Join(t.TempDir(), "marks")
	_ = os.MkdirAll(marks, 0o755)
	for _, name := range []string{"oom", "node-notready"} {
		dir := filepath.Join(w.scenarios, name)
		_ = os.MkdirAll(dir, 0o755)
		_ = os.WriteFile(filepath.Join(dir, "inject.sh"), []byte("#!/bin/sh\ntouch "+filepath.Join(marks, name)+"\n"), 0o755)
		_ = os.WriteFile(filepath.Join(dir, "clear.sh"), []byte("#!/bin/sh\nrm "+filepath.Join(marks, name)+"\n"), 0o755)
		_ = os.WriteFile(filepath.Join(dir, "expected.md"), []byte("Alert: `LabPodOOMKilled`\n\n| Fact | Value |\n|---|---|\n| object | cart |\n| limit | 32Mi |\n| fix (not applied) | raise |\n"), 0o644)
	}
	t.Setenv("TYPESAFE_API_KEY", "k")
	return w
}

// injected marks an injection so the fake control plane shows the
// incident; the scripts cannot reach it, so the test watches the marks.
func (w *world) armed() {
	w.mu.Lock()
	w.injections++
	w.mu.Unlock()
}

func TestRunScoreAndReport(t *testing.T) {
	w := newWorld(t)
	w.armed()
	ctx := context.Background()
	common := []string{"--socket", w.cp.URL, "--jev-endpoint", w.jev.URL, "--out", w.out}
	// The settle step is kubectl against a cluster that does not exist, so
	// the lab reports it; the run keeps going and the result carries it.
	err := runCommand(ctx, append(common, "--scenarios", w.scenarios, "--only", "oom", "--poll", "1ms", "--timeout", "1s", "--context", "kind-nowhere"))
	if err == nil || !strings.Contains(err.Error(), "does not clear") {
		t.Fatalf("one scenario of ten: %v", err)
	}
	data, readErr := os.ReadFile(filepath.Join(w.out, "oom.json"))
	if readErr != nil || !strings.Contains(string(data), `"Scenario": "oom"`) || !strings.Contains(string(data), "partly") == false {
		t.Fatalf("%s %v", data, readErr)
	}
	if err := reportCommand([]string{"--out", w.out}); err == nil || !strings.Contains(err.Error(), "does not clear") {
		t.Fatalf("report: %v", err)
	}
	if err := reportCommand([]string{"--out", filepath.Join(w.out, "empty")}); err == nil || !strings.Contains(err.Error(), "no results") {
		t.Fatalf("empty report: %v", err)
	}
	expected := filepath.Join(w.scenarios, "oom", "expected.md")
	if err := scoreCommand(ctx, append(common, "--expected", expected, "--incident", "inc-1")); err != nil {
		t.Fatalf("score by incident: %v", err)
	}
	diagnosis := filepath.Join(t.TempDir(), "d.txt")
	_ = os.WriteFile(diagnosis, []byte("cart is OOMKilled"), 0o644)
	if err := scoreCommand(ctx, append(common, "--expected", expected, "--diagnosis", diagnosis)); err != nil {
		t.Fatalf("score by file: %v", err)
	}
}

func TestCommandsRefuseBadArguments(t *testing.T) {
	w := newWorld(t)
	ctx := context.Background()
	common := []string{"--socket", w.cp.URL, "--jev-endpoint", w.jev.URL, "--out", w.out}
	cases := []struct {
		name string
		err  error
	}{
		{"no key", func() error {
			t.Setenv("TYPESAFE_API_KEY", "")
			defer t.Setenv("TYPESAFE_API_KEY", "k")
			return runCommand(ctx, common)
		}()},
		{"bad flag", runCommand(ctx, []string{"--nope"})},
		{"unknown scenario", runCommand(ctx, append(common, "--scenarios", w.scenarios, "--only", "nope"))},
		{"no scenarios dir", runCommand(ctx, append(common, "--scenarios", "/nowhere"))},
		{"soak not run", runCommand(ctx, append(common, "--scenarios", w.scenarios, "--only", "oom", "--soak", "node-notready", "--soak-for", "1m"))},
		{"score needs expected", scoreCommand(ctx, common)},
		{"score both sources", scoreCommand(ctx, append(common, "--expected", "x", "--incident", "i", "--diagnosis", "d"))},
		{"score no expected file", scoreCommand(ctx, append(common, "--expected", "/nowhere/expected.md", "--incident", "i"))},
		{"score bad expected", scoreCommand(ctx, append(common, "--expected", filepath.Join(w.scenarios, "oom", "inject.sh"), "--incident", "i"))},
		{"score no diagnosis file", scoreCommand(ctx, append(common, "--expected", filepath.Join(w.scenarios, "oom", "expected.md"), "--diagnosis", "/nowhere"))},
		{"score no incident", scoreCommand(ctx, append([]string{"--socket", "http://127.0.0.1:1"}, "--expected", filepath.Join(w.scenarios, "oom", "expected.md"), "--incident", "i"))},
		{"score bad flag", scoreCommand(ctx, []string{"--nope"})},
		{"report bad flag", reportCommand([]string{"--nope"})},
		{"report broken file", func() error {
			dir := t.TempDir()
			_ = os.WriteFile(filepath.Join(dir, "x.json"), []byte("{"), 0o644)
			return reportCommand([]string{"--out", dir})
		}()},
	}
	for _, c := range cases {
		if c.err == nil {
			t.Errorf("%s: accepted", c.name)
		}
	}
}

func TestChooseScenariosPutsNodeNotReadyLast(t *testing.T) {
	w := newWorld(t)
	names, err := chooseScenarios(shell.Lab{Dir: w.scenarios}, "")
	if err != nil || strings.Join(names, ",") != "oom,node-notready" {
		t.Fatalf("%v %v", names, err)
	}
	names, err = chooseScenarios(shell.Lab{Dir: w.scenarios}, "node-notready, oom")
	if err != nil || strings.Join(names, ",") != "node-notready,oom" {
		t.Fatalf("%v %v", names, err)
	}
}
