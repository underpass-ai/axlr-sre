// Command sre-judge runs the lab's scenarios against the bots and has Jev
// judge each diagnosis: phase 0's exit numbers, measured.
package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
)

const usage = `usage: sre-judge <command> [flags]

  run     inject each scenario, wait for its incident to settle, judge the diagnosis with Jev, clear;
          one JSON per scenario in --out, the tables at the end; exit 1 when phase 0's goals fail
  score   judge one incident's diagnosis (or a text file) against a scenario's expected.md
  report  rebuild the tables from the JSON files in --out; exit 1 when the goals fail

The Jev key is TYPESAFE_API_KEY in the environment; nothing else reads it.
`

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "run":
		err = runCommand(ctx, os.Args[2:])
	case "score":
		err = scoreCommand(ctx, os.Args[2:])
	case "report":
		err = reportCommand(os.Args[2:])
	case "-h", "--help", "help":
		fmt.Print(usage)
	default:
		fmt.Fprintf(os.Stderr, "sre-judge: unknown command %q\n%s", os.Args[1], usage)
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "sre-judge:", err)
		os.Exit(1)
	}
}
