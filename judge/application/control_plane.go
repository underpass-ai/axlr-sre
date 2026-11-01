package application

import (
	"context"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// ControlPlane is as much of axlr-bots' operator API as the judge reads.
type ControlPlane interface {
	Incidents(ctx context.Context, tenant string) ([]domain.Incident, error)
	Incident(ctx context.Context, id string) (domain.Incident, error)
	Events(ctx context.Context, bot string, limit int) (domain.Timeline, error)
	Task(ctx context.Context, id string) (domain.Task, error)
	Executions(ctx context.Context, taskID string) ([]domain.Execution, error)
}
