// Package controlplane reads axlr-bots' operator API over its Unix socket,
// as the axlr-bots CLI does: no token, the socket's permissions decide.
// Its one write is a task for the fix bot.
package controlplane

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

const maxBodyBytes = 4 << 20

// Client is the operator API over the socket.
type Client struct {
	http http.Client
	base string
}

// NewClient dials the socket for every request. A base URL (http://…)
// instead of a socket path is for tests.
func NewClient(socket string, timeout time.Duration) *Client {
	if timeout == 0 {
		timeout = 30 * time.Second
	}
	client := Client{base: "http://axlr-bots"}
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		var d net.Dialer
		return d.DialContext(ctx, "unix", socket)
	}}
	if u, err := url.Parse(socket); err == nil && (u.Scheme == "http" || u.Scheme == "https") {
		client.base, transport = socket, nil
	}
	client.http = http.Client{Timeout: timeout}
	if transport != nil {
		client.http.Transport = transport
	}
	return &client
}

type incidentWire struct {
	ID        string   `json:"id"`
	GroupKey  string   `json:"groupKey"`
	Status    string   `json:"status"`
	Cycle     int      `json:"cycle"`
	Repeats   int      `json:"repeats"`
	CostUSD   float64  `json:"costUsd"`
	Diagnosis string   `json:"diagnosis"`
	Proposal  string   `json:"proposal"`
	TaskIDs   []string `json:"taskIds"`
	Tasks     []struct {
		TaskID string `json:"taskId"`
		BotID  string `json:"botId"`
		Phase  string `json:"phase"`
		Cycle  int    `json:"cycle"`
	} `json:"tasks"`
	OpenedAt  time.Time `json:"openedAt"`
	UpdatedAt time.Time `json:"updatedAt"`
}

func (w incidentWire) incident() domain.Incident {
	incident := domain.Incident{ID: w.ID, GroupKey: w.GroupKey, Status: w.Status, Cycle: w.Cycle, Repeats: w.Repeats,
		CostUSD: w.CostUSD, Diagnosis: w.Diagnosis, Proposal: w.Proposal, TaskIDs: w.TaskIDs, OpenedAt: w.OpenedAt, UpdatedAt: w.UpdatedAt}
	for _, t := range w.Tasks {
		incident.Tasks = append(incident.Tasks, domain.IncidentTask{TaskID: t.TaskID, BotID: t.BotID, Phase: t.Phase, Cycle: t.Cycle})
	}
	return incident
}

// Incidents lists the tenant's incidents (the API's newest-first page of
// up to 100).
func (c *Client) Incidents(ctx context.Context, tenant string) ([]domain.Incident, error) {
	var wire []incidentWire
	query := url.Values{"tenant": {tenant}, "limit": {"100"}}
	if err := c.get(ctx, "/v1/incidents?"+query.Encode(), &wire); err != nil {
		return nil, err
	}
	incidents := make([]domain.Incident, 0, len(wire))
	for _, w := range wire {
		incidents = append(incidents, w.incident())
	}
	return incidents, nil
}

// Incident reads one incident.
func (c *Client) Incident(ctx context.Context, id string) (domain.Incident, error) {
	var wire incidentWire
	if err := c.get(ctx, "/v1/incidents/"+url.PathEscape(id), &wire); err != nil {
		return domain.Incident{}, err
	}
	return wire.incident(), nil
}

// Events reads the bot's newest events, oldest first, and keeps those of
// incidents.
func (c *Client) Events(ctx context.Context, bot string, limit int) (domain.Timeline, error) {
	var wire []struct {
		Type       string    `json:"type"`
		IncidentID string    `json:"incidentId"`
		CreatedAt  time.Time `json:"createdAt"`
	}
	if err := c.get(ctx, "/v1/bots/"+url.PathEscape(bot)+"/events?limit="+strconv.Itoa(limit), &wire); err != nil {
		return nil, err
	}
	var timeline domain.Timeline
	for _, w := range wire {
		if w.IncidentID != "" {
			timeline = append(timeline, domain.Event{Type: w.Type, IncidentID: w.IncidentID, CreatedAt: w.CreatedAt})
		}
	}
	return timeline, nil
}

// Task reads one task.
func (c *Client) Task(ctx context.Context, id string) (domain.Task, error) {
	var wire struct {
		ID           string    `json:"id"`
		Status       string    `json:"status"`
		ExecutionIDs []string  `json:"executionIds"`
		CreatedAt    time.Time `json:"createdAt"`
	}
	if err := c.get(ctx, "/v1/tasks/"+url.PathEscape(id), &wire); err != nil {
		return domain.Task{}, err
	}
	return domain.Task{ID: wire.ID, Status: wire.Status, ExecutionIDs: wire.ExecutionIDs, CreatedAt: wire.CreatedAt}, nil
}

// Executions reads a task's executions with what each cost and answered.
func (c *Client) Executions(ctx context.Context, taskID string) ([]domain.Execution, error) {
	var wire []struct {
		ID    string `json:"id"`
		Usage struct {
			CostUSD float64 `json:"costUsd"`
		} `json:"usage"`
		Result struct {
			Answer string `json:"answer"`
		} `json:"result"`
	}
	if err := c.get(ctx, "/v1/tasks/"+url.PathEscape(taskID)+"/executions", &wire); err != nil {
		return nil, err
	}
	executions := make([]domain.Execution, 0, len(wire))
	for _, w := range wire {
		executions = append(executions, domain.Execution{ID: w.ID, CostUSD: w.Usage.CostUSD, Answer: w.Result.Answer})
	}
	return executions, nil
}

func (c *Client) get(ctx context.Context, path string, into any) error {
	return c.do(ctx, http.MethodGet, path, nil, into)
}

func (c *Client) do(ctx context.Context, method, path string, body io.Reader, into any) error {
	request, err := http.NewRequestWithContext(ctx, method, c.base+path, body)
	if err != nil {
		return err
	}
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	response, err := c.http.Do(request)
	if err != nil {
		return fmt.Errorf("control plane: %w", err)
	}
	defer response.Body.Close()
	data, err := io.ReadAll(io.LimitReader(response.Body, maxBodyBytes))
	if err != nil {
		return fmt.Errorf("control plane: %w", err)
	}
	if response.StatusCode != http.StatusOK && response.StatusCode != http.StatusCreated {
		return fmt.Errorf("control plane answered HTTP %d for %s", response.StatusCode, path)
	}
	if err := json.Unmarshal(data, into); err != nil {
		return errors.New("control plane answered something that is not the JSON expected for " + path)
	}
	return nil
}
