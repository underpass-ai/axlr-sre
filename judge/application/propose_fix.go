package application

import (
	"context"
	"errors"
	"fmt"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// ProposeFix stands in for the control plane's fix phase (design §4),
// which axlr-bots does not launch yet: for a diagnosed incident of a
// write-node scenario it submits a task to the fix bot with triage's
// diagnosis, waits for the task to end and reads its answer and cost.
type ProposeFix struct {
	ControlPlane ControlPlane
	Clock        Clock
	// Bot is the fix bot's id.
	Bot     string
	Timeout time.Duration
	Poll    time.Duration
}

// ErrProposalTimeout says the fix task did not end in time.
var ErrProposalTimeout = errors.New("the fix task did not end in time")

// Run proposes a fix for the incident. The task's key names the incident
// and its cycle, so a judge that resumes finds the same task.
func (uc ProposeFix) Run(ctx context.Context, incident domain.Incident) (domain.Proposal, error) {
	proposal := domain.Proposal{SubmittedAt: uc.Clock.Now()}
	key := fmt.Sprintf("sre-judge:%s:%d:fix", incident.ID, incident.Cycle)
	id, err := uc.ControlPlane.SubmitTask(ctx, uc.Bot, domain.FixObjective(incident), key)
	if err != nil {
		return proposal, fmt.Errorf("submit to %s: %w", uc.Bot, err)
	}
	proposal.TaskID = id
	deadline := proposal.SubmittedAt.Add(uc.Timeout)
	for {
		task, err := uc.ControlPlane.Task(ctx, id)
		if err != nil {
			return proposal, err
		}
		if domain.TaskEnded(task.Status) {
			proposal.Status, proposal.EndedAt = task.Status, uc.Clock.Now()
			break
		}
		if !uc.Clock.Now().Before(deadline) {
			proposal.Status = task.Status
			return proposal, ErrProposalTimeout
		}
		if err := uc.Clock.Sleep(ctx, uc.Poll); err != nil {
			return proposal, err
		}
	}
	executions, err := uc.ControlPlane.Executions(ctx, id)
	if err != nil {
		return proposal, err
	}
	for _, execution := range executions {
		proposal.CostUSD += execution.CostUSD
		if execution.Answer != "" {
			proposal.Answer = execution.Answer
		}
	}
	return proposal, nil
}
