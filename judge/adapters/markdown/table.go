// Package markdown renders a run as the tables the README keeps.
package markdown

import (
	"fmt"
	"strings"
	"time"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Render writes the run: the totals against the goals, one row per
// scenario, and each scenario's facts.
func Render(run domain.Run, goals domain.Goals) string {
	var b strings.Builder
	fmt.Fprintf(&b, "| Measure | Result | Phase 0 goal |\n|---|---|---|\n")
	fmt.Fprintf(&b, "| scenarios that fired their alert and opened one incident | %d / %d | %d / %d |\n", run.Fired(), len(run.Results), goals.Scenarios, goals.Scenarios)
	fmt.Fprintf(&b, "| causes Jev found stated in full (partly) | %d / %d (%d) | %d / %d |\n", run.Correct(), len(run.Results), run.Partly(), goals.Correct, goals.Scenarios)
	fmt.Fprintf(&b, "| duplicate incidents | %d | %d |\n", run.Duplicates(), goals.Duplicates)
	fmt.Fprintf(&b, "| Open → Diagnosed | %s | < %s |\n", spread(run), goals.OpenToSettled)
	fmt.Fprintf(&b, "| cost of the whole run, children included | $%.3f | < $%.0f |\n", run.CostUSD(), goals.CostPerRun)
	fmt.Fprintf(&b, "| cost of judging (Jev) | $%.4f | |\n", run.JevCostUSD())
	fmt.Fprintf(&b, "| passes | %t | |\n\n", run.Passes(goals))

	fmt.Fprintf(&b, "| Scenario | Alert | Trigger answers | Inject → settled | Open → settled | Cost | Resends | Duplicates | Other alerts | Facts stated | Fix proposed | Cause |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n")
	for _, r := range run.Results {
		fmt.Fprintf(&b, "| %s | `%s` | %s | %s | %s | $%.4f | %d | %d | %s | %s | %s | **%s** |\n",
			r.Scenario, r.Alert, answers(r), seconds(r.InjectToSettled()), seconds(r.OpenToSettled), r.CostUSD, r.Repeats,
			r.Duplicates, list(r.OtherAlerts), facts(r), yesNo(r.Score.FixStated()), outcome(r))
	}
	for _, r := range run.Results {
		if len(r.Score.Verdicts) == 0 {
			continue
		}
		fmt.Fprintf(&b, "\n### %s\n\n| Fact | Expected | Jev | p |\n|---|---|---|---|\n", r.Scenario)
		for _, v := range r.Score.Verdicts {
			fmt.Fprintf(&b, "| %s | %s | %s | %.2f |\n", v.Fact.Name, v.Fact.Value, v.Mark(), v.Choice.Probability)
		}
	}
	return b.String()
}

func answers(r domain.Result) string {
	var parts []string
	if r.Opened > 0 {
		parts = append(parts, "opened")
	}
	if r.Reopened > 0 {
		parts = append(parts, "reopened")
	}
	if r.Repeated > 0 {
		parts = append(parts, fmt.Sprintf("repeated ×%d", r.Repeated))
	}
	if len(parts) == 0 {
		return "—"
	}
	return strings.Join(parts, ", ")
}

func seconds(d time.Duration) string {
	if d == 0 {
		return "—"
	}
	return fmt.Sprintf("%d s", int(d.Round(time.Second).Seconds()))
}

func list(items []string) string {
	if len(items) == 0 {
		return "—"
	}
	return strings.Join(items, ", ")
}

func facts(r domain.Result) string {
	if r.Score.CauseTotal() == 0 {
		return "—"
	}
	s := fmt.Sprintf("%d / %d", r.Score.CauseStated(), r.Score.CauseTotal())
	if missing := r.Score.MissingFacts(); len(missing) > 0 {
		s += " (not " + strings.Join(missing, ", ") + ")"
	}
	return s
}

func yesNo(b bool) string {
	if b {
		return "yes"
	}
	return "no"
}

func outcome(r domain.Result) string {
	if r.Error != "" {
		return string(r.Outcome()) + ": " + r.Error
	}
	return string(r.Outcome())
}

func spread(run domain.Run) string {
	var lo, hi time.Duration
	for _, r := range run.Results {
		d := r.OpenToSettled
		if d == 0 {
			continue
		}
		if lo == 0 || d < lo {
			lo = d
		}
		if d > hi {
			hi = d
		}
	}
	if hi == 0 {
		return "—"
	}
	return fmt.Sprintf("%s–%s", seconds(lo), seconds(hi))
}
