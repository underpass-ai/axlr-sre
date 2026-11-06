package application

import (
	"context"
	"errors"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// AwaitProposal follows the control plane's fix phase (design §4) for a
// diagnosed incident of a write-node scenario: axlr-bots gives the fix
// task to the teammate that declares the fix phase, and the incident
// leaves Diagnosed when that task ends, Proposed with its answer or
// Escalated. The judge only reads: it submits nothing.
type AwaitProposal struct {
	ControlPlane ControlPlane
	Clock        Clock
	Timeout      time.Duration
	Poll         time.Duration
}

// ErrProposalTimeout says the incident did not leave Diagnosed in time.
var ErrProposalTimeout = errors.New("the incident did not leave Diagnosed in time")

// ErrNoFixPhase says the incident left Diagnosed, or the time ran out,
// and the control plane had launched no fix task: no teammate of triage
// declares the fix phase, two do, or the fix bot is paused.
var ErrNoFixPhase = errors.New(`the control plane launched no fix task: fix needs the trigger {"type": "incident", "phase": "fix"} and triage's team`)

// Run waits for the incident's fix phase and reads its proposal, and
// answers the incident as it then stands, with every phase's cost.
func (uc AwaitProposal) Run(ctx context.Context, incident domain.Incident) (domain.Proposal, domain.Incident, error) {
	var proposal domain.Proposal
	deadline := uc.Clock.Now().Add(uc.Timeout)
	current := incident
	for current.Status == domain.IncidentDiagnosed {
		if !uc.Clock.Now().Before(deadline) {
			if current.FixTask() == "" {
				return proposal, current, ErrNoFixPhase
			}
			proposal.TaskID = current.FixTask()
			return proposal, current, ErrProposalTimeout
		}
		if err := uc.Clock.Sleep(ctx, uc.Poll); err != nil {
			return proposal, current, err
		}
		var err error
		if current, err = uc.ControlPlane.Incident(ctx, incident.ID); err != nil {
			return proposal, current, err
		}
	}
	proposal.TaskID, proposal.EndedAt = current.FixTask(), uc.Clock.Now()
	if proposal.TaskID == "" {
		return proposal, current, ErrNoFixPhase
	}
	task, err := uc.ControlPlane.Task(ctx, proposal.TaskID)
	if err != nil {
		return proposal, current, err
	}
	proposal.Status, proposal.SubmittedAt = task.Status, task.CreatedAt
	executions, err := uc.ControlPlane.Executions(ctx, proposal.TaskID)
	if err != nil {
		return proposal, current, err
	}
	for _, execution := range executions {
		proposal.CostUSD += execution.CostUSD
	}
	proposal.Answer = current.Proposal
	return proposal, current, nil
}
