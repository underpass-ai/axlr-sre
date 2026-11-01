package application

import (
	"context"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// IncidentCost adds to an incident's own cost what its delegated children
// cost: axlr-bots counts only the triage task's executions (PR 3's
// finding), so the judge reads the children's executions itself.
type IncidentCost struct {
	ControlPlane ControlPlane
}

// Of is the incident's cost with every child of its current task added,
// grandchildren included.
func (uc IncidentCost) Of(ctx context.Context, incident domain.Incident) (float64, error) {
	cost := incident.CostUSD
	if root := incident.CurrentTask(); root != "" {
		task, err := uc.ControlPlane.Task(ctx, root)
		if err != nil {
			return 0, err
		}
		for _, child := range task.Children {
			childCost, err := uc.costOfTask(ctx, child, 0)
			if err != nil {
				return 0, err
			}
			cost += childCost
		}
	}
	return cost, nil
}

const maxDelegationDepth = 8

func (uc IncidentCost) costOfTask(ctx context.Context, id string, depth int) (float64, error) {
	if depth > maxDelegationDepth {
		return 0, nil
	}
	executions, err := uc.ControlPlane.Executions(ctx, id)
	if err != nil {
		return 0, err
	}
	cost := 0.0
	for _, e := range executions {
		cost += e.CostUSD
	}
	task, err := uc.ControlPlane.Task(ctx, id)
	if err != nil {
		return 0, err
	}
	for _, child := range task.Children {
		childCost, err := uc.costOfTask(ctx, child, depth+1)
		if err != nil {
			return 0, err
		}
		cost += childCost
	}
	return cost, nil
}
