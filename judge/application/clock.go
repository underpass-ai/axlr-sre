package application

import (
	"context"
	"time"
)

// Clock is time as the use cases see it; tests pass their own.
type Clock interface {
	Now() time.Time
	// Sleep waits, or returns the context's error first.
	Sleep(ctx context.Context, d time.Duration) error
}

// SystemClock is the wall clock.
type SystemClock struct{}

// Now is time.Now.
func (SystemClock) Now() time.Time { return time.Now() }

// Sleep waits with time.After.
func (SystemClock) Sleep(ctx context.Context, d time.Duration) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(d):
		return nil
	}
}
