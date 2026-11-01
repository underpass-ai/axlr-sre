package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/adapters/markdown"
	"github.com/underpass-ai/axlr-sre/judge/adapters/results"
	"github.com/underpass-ai/axlr-sre/judge/adapters/shell"
	"github.com/underpass-ai/axlr-sre/judge/application"
	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// lastScenario is run last: it stops a worker's kubelet, and the faults
// before it schedule onto that worker.
const lastScenario = "node-notready"

func runCommand(ctx context.Context, args []string) error {
	flags := flag.NewFlagSet("run", flag.ContinueOnError)
	var c common
	c.bind(flags)
	scenariosDir := flags.String("scenarios", "scenarios", "the scenarios directory")
	only := flags.String("only", "", "comma-separated scenarios to run; default every one, node-notready last")
	kubeContext := flags.String("context", "kind-sre-lab", "the kubectl context")
	tenant := flags.String("tenant", "ops", "the bots' tenant")
	bot := flags.String("bot", "triage", "the bot whose timeline holds the incident events")
	timeout := flags.Duration("timeout", 15*time.Minute, "how long an incident may stay Open")
	poll := flags.Duration("poll", 5*time.Second, "how often the incidents are read")
	soak := flags.String("soak", "", "a scenario left firing after its diagnosis, to count the resends")
	soakFor := flags.Duration("soak-for", 0, "how long the soaked scenario stays firing after its diagnosis")
	if err := flags.Parse(args); err != nil {
		return err
	}
	judge, err := c.judge()
	if err != nil {
		return err
	}
	lab := shell.Lab{Dir: *scenariosDir, Context: *kubeContext, Stdout: os.Stdout}
	scenarios, err := chooseScenarios(lab, *only)
	if err != nil {
		return err
	}
	if *soak != "" && !contains(scenarios, *soak) {
		return fmt.Errorf("the soak scenario %q is not among the scenarios to run", *soak)
	}
	cp := c.controlPlane()
	clock := application.SystemClock{}
	scenario := application.JudgeScenario{
		Lab: lab, ControlPlane: cp, Clock: clock,
		Score:    application.ScoreDiagnosis{Judge: judge},
		Cost:     application.IncidentCost{ControlPlane: cp},
		Settings: application.Settings{Tenant: *tenant, Bot: *bot, Timeout: *timeout, Poll: *poll, EventLimit: 1000},
	}
	store := results.Store{Dir: c.out}
	runner := application.RunScenarios{
		Scenario: scenario, Clock: clock,
		Soak: application.Soak{Scenario: *soak, For: *soakFor},
		Keep: store.Keep,
		Say:  func(line string) { fmt.Printf("%s  %s\n", time.Now().UTC().Format("15:04:05"), line) },
	}
	if _, err := runner.Run(ctx, scenarios); err != nil {
		return err
	}
	run, err := store.Load()
	if err != nil {
		return err
	}
	fmt.Print("\n" + markdown.Render(run, domain.Phase0))
	if !run.Passes(domain.Phase0) {
		return errors.New("the run does not clear phase 0's goals")
	}
	return nil
}

func chooseScenarios(lab shell.Lab, only string) ([]string, error) {
	all, err := lab.Scenarios()
	if err != nil {
		return nil, err
	}
	if only != "" {
		var chosen []string
		for _, name := range strings.Split(only, ",") {
			name = strings.TrimSpace(name)
			if !contains(all, name) {
				return nil, fmt.Errorf("no scenario %q under the scenarios directory", name)
			}
			chosen = append(chosen, name)
		}
		return chosen, nil
	}
	var ordered []string
	for _, name := range all {
		if name != lastScenario {
			ordered = append(ordered, name)
		}
	}
	if contains(all, lastScenario) {
		ordered = append(ordered, lastScenario)
	}
	return ordered, nil
}

func contains(names []string, name string) bool {
	for _, n := range names {
		if n == name {
			return true
		}
	}
	return false
}
