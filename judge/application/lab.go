package application

import (
	"context"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Lab is the kind cluster and the scenarios' scripts.
type Lab interface {
	Scenarios() ([]string, error)
	Expected(scenario string) (domain.Expected, error)
	Inject(ctx context.Context, scenario string) error
	// Clear runs the scenario's clear.sh and waits for the cluster to
	// settle (every node Ready).
	Clear(ctx context.Context, scenario string) error
}
