// Package application runs the scenarios and judges them: it talks to
// the lab, the control plane and Jev through ports, and keeps no state
// of its own.
package application

import (
	"context"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Judge answers one question with a choice: Jev, through TypeSafe.
type Judge interface {
	Choose(ctx context.Context, question domain.Question) (domain.Choice, error)
}
