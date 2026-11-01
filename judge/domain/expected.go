package domain

import (
	"errors"
	"regexp"
	"strings"
)

// Expected is what a scenario's expected.md promises: the alert that
// fires and the facts a correct diagnosis states.
type Expected struct {
	Scenario string
	Alert    string
	Facts    []Fact
}

var alertLine = regexp.MustCompile("Alert: `([^`]+)`")

// ParseExpected reads an expected.md: the alert from its "Alert: `X`" line
// and one fact per row of its two-column table, header and rule skipped.
func ParseExpected(scenario, markdown string) (Expected, error) {
	expected := Expected{Scenario: scenario}
	if m := alertLine.FindStringSubmatch(markdown); m != nil {
		expected.Alert = m[1]
	}
	for _, line := range strings.Split(markdown, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "|") {
			continue
		}
		cells := strings.Split(strings.Trim(line, "|"), "|")
		if len(cells) != 2 {
			continue
		}
		name, value := strings.TrimSpace(cells[0]), strings.TrimSpace(cells[1])
		if name == "" || value == "" || strings.EqualFold(name, "Fact") || strings.Trim(name, "-: ") == "" {
			continue
		}
		expected.Facts = append(expected.Facts, Fact{Name: name, Value: value})
	}
	switch {
	case expected.Alert == "":
		return Expected{}, errors.New(scenario + ": expected.md names no alert (Alert: `…`)")
	case len(expected.CauseFacts()) == 0:
		return Expected{}, errors.New(scenario + ": expected.md has no fact table")
	}
	return expected, nil
}

// CauseFacts are the facts that make the cause: every row but the fix.
func (e Expected) CauseFacts() []Fact {
	var facts []Fact
	for _, fact := range e.Facts {
		if !fact.IsFix() {
			facts = append(facts, fact)
		}
	}
	return facts
}
