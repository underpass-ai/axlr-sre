package main

import (
	"errors"
	"flag"
	"fmt"

	"github.com/underpass-ai/axlr-sre/judge/adapters/markdown"
	"github.com/underpass-ai/axlr-sre/judge/adapters/results"
)

func reportCommand(args []string) error {
	flags := flag.NewFlagSet("report", flag.ContinueOnError)
	var c common
	c.bind(flags)
	if err := flags.Parse(args); err != nil {
		return err
	}
	run, err := results.Store{Dir: c.out}.Load()
	if err != nil {
		return err
	}
	if len(run.Results) == 0 {
		return errors.New("no results under " + c.out)
	}
	fmt.Print(markdown.RenderByNode(run))
	if !run.PassesByNode() {
		return errors.New("the run does not clear its nodes' goals")
	}
	return nil
}
