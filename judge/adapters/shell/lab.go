// Package shell is the lab as scripts: scenarios/<name>/{inject,clear}.sh
// and expected.md on disk, kubectl to let the cluster settle.
package shell

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Lab is the scenarios directory and the kubectl context.
type Lab struct {
	Dir     string
	Context string
	// Settle is the command run after clear.sh; empty means kubectl wait
	// for every node Ready. Tests set their own.
	Settle []string
	Stdout *os.File
}

// WriteNodeMarker is the file that keeps a scenario out of the default
// run: only the write node (sre-rw) can diagnose it. drift's evidence is
// kubectl diff, a server dry run that needs patch, which the read node's
// ServiceAccount does not have.
const WriteNodeMarker = "needs-write-node"

// NeedsWriteNode reports a scenario that holds the WriteNodeMarker.
func (l Lab) NeedsWriteNode(scenario string) bool {
	_, err := os.Stat(filepath.Join(l.Dir, scenario, WriteNodeMarker))
	return err == nil
}

// Scenarios lists the directories holding an inject.sh, in name order.
func (l Lab) Scenarios() ([]string, error) {
	entries, err := os.ReadDir(l.Dir)
	if err != nil {
		return nil, err
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() {
			if _, err := os.Stat(filepath.Join(l.Dir, e.Name(), "inject.sh")); err == nil {
				names = append(names, e.Name())
			}
		}
	}
	sort.Strings(names)
	return names, nil
}

// Expected parses the scenario's expected.md.
func (l Lab) Expected(scenario string) (domain.Expected, error) {
	data, err := os.ReadFile(filepath.Join(l.Dir, scenario, "expected.md"))
	if err != nil {
		return domain.Expected{}, err
	}
	return domain.ParseExpected(scenario, string(data))
}

// Inject runs the scenario's inject.sh.
func (l Lab) Inject(ctx context.Context, scenario string) error {
	return l.run(ctx, filepath.Join(l.Dir, scenario, "inject.sh"))
}

// Clear runs the scenario's clear.sh, then waits for the cluster to
// settle.
func (l Lab) Clear(ctx context.Context, scenario string) error {
	if err := l.run(ctx, filepath.Join(l.Dir, scenario, "clear.sh")); err != nil {
		return err
	}
	settle := l.Settle
	if len(settle) == 0 {
		settle = []string{"kubectl", "--context", l.Context, "wait", "--for=condition=Ready", "nodes", "--all", "--timeout=180s"}
	}
	return l.run(ctx, settle[0], settle[1:]...)
}

func (l Lab) run(ctx context.Context, program string, args ...string) error {
	if strings.HasSuffix(program, ".sh") {
		if _, err := os.Stat(program); err != nil {
			return err
		}
		args, program = append([]string{program}, args...), "bash"
	}
	cmd := exec.CommandContext(ctx, program, args...)
	cmd.Stdout, cmd.Stderr = l.Stdout, l.Stdout
	if err := cmd.Run(); err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) {
			return fmt.Errorf("%s %s exited %d", program, strings.Join(args, " "), exit.ExitCode())
		}
		return err
	}
	return nil
}
