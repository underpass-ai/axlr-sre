package main

import (
	"errors"
	"flag"
	"os"
	"path/filepath"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/adapters/controlplane"
	"github.com/underpass-ai/axlr-sre/judge/adapters/jev"
	"github.com/underpass-ai/axlr-sre/judge/application"
)

// common are the flags every command shares.
type common struct {
	socket, out, model, jevEndpoint string
}

func (c *common) bind(flags *flag.FlagSet) {
	home, _ := os.UserHomeDir()
	flags.StringVar(&c.socket, "socket", filepath.Join(home, ".local/state/axlr-bots/axlr-bots.sock"), "the control plane's socket (or an http:// base for tests)")
	flags.StringVar(&c.out, "out", "judge/results", "the directory of one JSON per scenario")
	flags.StringVar(&c.model, "model", jev.DefaultModel, "the Jev version, pinned")
	flags.StringVar(&c.jevEndpoint, "jev-endpoint", "", "TypeSafe's endpoint; empty means the real one (tests set their own)")
}

func (c common) controlPlane() *controlplane.Client {
	return controlplane.NewClient(c.socket, 30*time.Second)
}

func (c common) judge() (application.Judge, error) {
	key := os.Getenv("TYPESAFE_API_KEY")
	if key == "" {
		return nil, errors.New("TYPESAFE_API_KEY is not set; the judge needs Jev")
	}
	return jev.NewClient(jev.Config{APIKey: key, Model: c.model, Endpoint: c.jevEndpoint})
}
