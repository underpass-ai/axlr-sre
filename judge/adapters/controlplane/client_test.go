package controlplane

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func api() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/incidents", func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("tenant") != "ops" || r.URL.Query().Get("limit") != "100" {
			http.Error(w, "query", http.StatusBadRequest)
			return
		}
		_, _ = w.Write([]byte(`[{"id":"inc-1","groupKey":"{}/{}:{alertname=\"A\"}","status":"Open","cycle":1,"repeats":2,"costUsd":0.01,"taskIds":["t1"],"openedAt":"2026-11-01T10:00:00Z","updatedAt":"2026-11-01T10:00:05Z"}]`))
	})
	mux.HandleFunc("GET /v1/incidents/{id}", func(w http.ResponseWriter, r *http.Request) {
		if r.PathValue("id") != "inc-1" {
			http.NotFound(w, r)
			return
		}
		_, _ = w.Write([]byte(`{"id":"inc-1","status":"Proposed","cycle":1,"diagnosis":"CAUSE: x","proposal":"PROPOSAL: apply","taskIds":["t1","t2"],` +
			`"tasks":[{"taskId":"t1","botId":"triage","phase":"triage","cycle":1},{"taskId":"t2","botId":"fix","phase":"fix","cycle":1}]}`))
	})
	mux.HandleFunc("GET /v1/bots/{id}/events", func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`[{"type":"bot.trigger.fired","createdAt":"2026-11-01T10:00:00Z"},{"type":"incident.opened","incidentId":"inc-1","createdAt":"2026-11-01T10:00:01Z"}]`))
	})
	mux.HandleFunc("GET /v1/tasks/{id}", func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"id":"t1","status":"Completed","children":["c1"],"executionIds":["e1"],"createdAt":"2026-11-01T10:00:02Z"}`))
	})
	mux.HandleFunc("GET /v1/tasks/{id}/executions", func(w http.ResponseWriter, r *http.Request) {
		if r.PathValue("id") == "broken" {
			_, _ = w.Write([]byte(`{"not":"a list"}`))
			return
		}
		_, _ = w.Write([]byte(`[{"id":"e1","usage":{"costUsd":0.002},"result":{"answer":"CAUSE: y"}}]`))
	})
	return mux
}

func TestClientReadsTheOperatorAPI(t *testing.T) {
	for name, client := range map[string]*Client{"http": overHTTP(t), "socket": overSocket(t)} {
		t.Run(name, func(t *testing.T) {
			ctx := context.Background()
			incidents, err := client.Incidents(ctx, "ops")
			if err != nil || len(incidents) != 1 || incidents[0].Repeats != 2 || incidents[0].Cycle != 1 || incidents[0].TaskIDs[0] != "t1" {
				t.Fatalf("%+v %v", incidents, err)
			}
			if incidents[0].OpenedAt != time.Date(2026, 11, 1, 10, 0, 0, 0, time.UTC) {
				t.Fatal(incidents[0].OpenedAt)
			}
			incident, err := client.Incident(ctx, "inc-1")
			if err != nil || incident.Diagnosis != "CAUSE: x" || incident.Proposal != "PROPOSAL: apply" || incident.FixTask() != "t2" ||
				incident.Tasks[0].Phase != "triage" || incident.Tasks[1].BotID != "fix" {
				t.Fatalf("%+v %v", incident, err)
			}
			if _, err := client.Incident(ctx, "inc-2"); err == nil || !strings.Contains(err.Error(), "404") {
				t.Fatalf("%v", err)
			}
			events, err := client.Events(ctx, "triage", 10)
			if err != nil || len(events) != 1 || events[0].IncidentID != "inc-1" {
				t.Fatalf("only incident events: %+v %v", events, err)
			}
			task, err := client.Task(ctx, "t1")
			if err != nil || task.Status != "Completed" || task.ExecutionIDs[0] != "e1" || !task.CreatedAt.Equal(time.Date(2026, 11, 1, 10, 0, 2, 0, time.UTC)) {
				t.Fatalf("%+v %v", task, err)
			}
			executions, err := client.Executions(ctx, "t1")
			if err != nil || len(executions) != 1 || executions[0].CostUSD != 0.002 || executions[0].Answer != "CAUSE: y" {
				t.Fatalf("%+v %v", executions, err)
			}
			if _, err := client.Executions(ctx, "broken"); err == nil || !strings.Contains(err.Error(), "not the JSON") {
				t.Fatalf("%v", err)
			}
		})
	}
	if _, err := NewClient("/nowhere/axlr-bots.sock", 0).Incidents(context.Background(), "ops"); err == nil {
		t.Fatal("a missing socket is an error")
	}
}

func overHTTP(t *testing.T) *Client {
	srv := httptest.NewServer(api())
	t.Cleanup(srv.Close)
	return NewClient(srv.URL, time.Second)
}

func overSocket(t *testing.T) *Client {
	socket := filepath.Join(t.TempDir(), "s.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	srv := &http.Server{Handler: api()}
	go func() { _ = srv.Serve(listener) }()
	t.Cleanup(func() { _ = srv.Close() })
	return NewClient(socket, time.Second)
}
