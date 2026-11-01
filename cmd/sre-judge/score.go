package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"github.com/underpass-ai/axlr-sre/judge/adapters/markdown"
	"github.com/underpass-ai/axlr-sre/judge/application"
	"github.com/underpass-ai/axlr-sre/judge/domain"
)

func scoreCommand(ctx context.Context, args []string) error {
	flags := flag.NewFlagSet("score", flag.ContinueOnError)
	var c common
	c.bind(flags)
	expectedPath := flags.String("expected", "", "the scenario's expected.md")
	incidentID := flags.String("incident", "", "the incident whose diagnosis is judged")
	diagnosisPath := flags.String("diagnosis", "", "a file holding the diagnosis, instead of an incident")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *expectedPath == "" || (*incidentID == "") == (*diagnosisPath == "") {
		return errors.New("score needs --expected and one of --incident or --diagnosis")
	}
	data, err := os.ReadFile(*expectedPath)
	if err != nil {
		return err
	}
	expected, err := domain.ParseExpected(filepath.Base(filepath.Dir(*expectedPath)), string(data))
	if err != nil {
		return err
	}
	diagnosis := ""
	if *diagnosisPath != "" {
		text, err := os.ReadFile(*diagnosisPath)
		if err != nil {
			return err
		}
		diagnosis = string(text)
	} else {
		incident, err := c.controlPlane().Incident(ctx, *incidentID)
		if err != nil {
			return err
		}
		diagnosis = incident.Diagnosis
	}
	judge, err := c.judge()
	if err != nil {
		return err
	}
	score, err := application.ScoreDiagnosis{Judge: judge}.Score(ctx, expected, diagnosis)
	if err != nil {
		return err
	}
	result := domain.Result{Scenario: expected.Scenario, Alert: expected.Alert, Status: domain.IncidentDiagnosed, Score: score}
	fmt.Print(markdown.Render(domain.NewRun([]domain.Result{result}), domain.Phase0))
	return nil
}
