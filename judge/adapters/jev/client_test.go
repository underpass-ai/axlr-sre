package jev

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func question() domain.Question {
	return domain.NewFactQuestion(domain.Fact{Name: "limit", Value: "32Mi"}, "the limit is 32Mi")
}

func server(t *testing.T, handler http.HandlerFunc) (*Client, *httptest.Server) {
	t.Helper()
	srv := httptest.NewServer(handler)
	t.Cleanup(srv.Close)
	client, err := NewClient(Config{APIKey: "k", Endpoint: srv.URL})
	if err != nil {
		t.Fatal(err)
	}
	return client, srv
}

func answer(model, choice string, probabilities map[string]any, tokens int) []byte {
	data, _ := json.Marshal(map[string]any{"model": model, "answers": map[string]any{"q": map[string]any{
		"type": "choice", "choice": choice, "probabilities": probabilities, "confidence": 0.9}}, "usage": map[string]any{"input_tokens": tokens}})
	return data
}

func TestChooseSendsTheQuestionAndReadsTheChoice(t *testing.T) {
	var got map[string]any
	client, _ := server(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer k" {
			t.Error("the key goes in the Authorization header")
		}
		_ = json.NewDecoder(r.Body).Decode(&got)
		_, _ = w.Write(answer(DefaultModel, "stated", map[string]any{"stated": 0.8, "missing": 0.15, "contradicted": 0.05}, 500))
	})
	choice, err := client.Choose(context.Background(), question())
	if err != nil {
		t.Fatal(err)
	}
	if choice.Option != "stated" || choice.Probability != 0.8 || choice.InputTokens != 500 || choice.Took <= 0 {
		t.Fatalf("%+v", choice)
	}
	q := got["questions"].(map[string]any)["q"].(map[string]any)
	if got["model"] != DefaultModel || q["type"] != "choice" || len(q["criteria"].(map[string]any)) != 3 {
		t.Fatalf("%v", got)
	}
	if client.Model() != DefaultModel {
		t.Fatal(client.Model())
	}
}

func TestChooseRetriesOn429AndRefusesTheRest(t *testing.T) {
	calls := 0
	client, _ := server(t, func(w http.ResponseWriter, r *http.Request) {
		calls++
		if calls == 1 {
			w.Header().Set("Retry-After", "0")
			w.WriteHeader(http.StatusTooManyRequests)
			return
		}
		_, _ = w.Write(answer(DefaultModel, "missing", map[string]any{"stated": 0.1, "missing": 0.8, "contradicted": 0.1}, 1))
	})
	if choice, err := client.Choose(context.Background(), question()); err != nil || choice.Option != "missing" || calls != 2 {
		t.Fatalf("%+v %v calls %d", choice, err, calls)
	}
	cases := map[string]http.HandlerFunc{
		"refused the API key": func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusUnauthorized) },
		"answered HTTP 500":   func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusInternalServerError) },
		"is not JSON":         func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write([]byte("nope")) },
		"another model": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write(answer("jev-9", "stated", map[string]any{"stated": 1, "missing": 0, "contradicted": 0}, 1))
		},
		"do not match": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write([]byte(`{"model":"` + DefaultModel + `","answers":{}}`))
		},
		"not a choice": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write([]byte(`{"model":"` + DefaultModel + `","answers":{"q":"x"}}`))
		},
		"among the options": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write(answer(DefaultModel, "stated", map[string]any{"stated": 1}, 1))
		},
		"do not cover": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write(answer(DefaultModel, "stated", map[string]any{"stated": nil, "missing": 0, "contradicted": 0}, 1))
		},
		"was not offered": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write(answer(DefaultModel, "maybe", map[string]any{"stated": 1, "missing": 0, "contradicted": 0}, 1))
		},
		"over 256 KiB": func(w http.ResponseWriter, r *http.Request) {
			_, _ = w.Write([]byte(strings.Repeat(" ", maxBodyBytes+2)))
		},
	}
	for want, handler := range cases {
		client, _ := server(t, handler)
		if _, err := client.Choose(context.Background(), question()); err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%s: %v", want, err)
		}
	}
	always429, _ := server(t, func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusTooManyRequests) })
	if _, err := always429.Choose(context.Background(), question()); err == nil || !strings.Contains(err.Error(), "429") {
		t.Fatalf("after the retries: %v", err)
	}
}

func TestChooseValidatesTheQuestionAndTheConfig(t *testing.T) {
	client, srv := server(t, func(w http.ResponseWriter, r *http.Request) { t.Error("nothing should be sent") })
	bad := []domain.Question{
		{State: "", Question: "q", Options: []string{"a", "b"}},
		{State: strings.Repeat("s", domain.MaxStateBytes+1), Question: "q", Options: []string{"a", "b"}},
		{State: "s", Question: "", Options: []string{"a", "b"}},
		{State: "s", Question: strings.Repeat("q", domain.MaxQuestionBytes+1), Options: []string{"a", "b"}},
		{State: "s", Question: "q", Options: []string{"a"}},
		{State: "s", Question: "q", Options: []string{"a", ""}},
		{State: "s", Question: "q", Options: []string{"a", "a"}},
	}
	for n, q := range bad {
		if _, err := client.Choose(context.Background(), q); err == nil {
			t.Errorf("question %d accepted", n)
		}
	}
	for _, config := range []Config{{APIKey: ""}, {APIKey: "k\nx"}, {APIKey: "k", Model: "jev-latest"}, {APIKey: "k", Model: "bad model"}} {
		if _, err := NewClient(config); err == nil {
			t.Errorf("config %+v accepted", config)
		}
	}
	srv.Close()
	if _, err := client.Choose(context.Background(), question()); err == nil || !strings.Contains(err.Error(), "did not answer") {
		t.Fatalf("closed server: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := client.Choose(ctx, question()); err != context.Canceled {
		t.Fatalf("cancelled: %v", err)
	}
}
