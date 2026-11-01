package domain

import (
	"strings"
	"testing"
)

const oomExpected = "# oom: expected facts\n\nAlert: `LabPodOOMKilled`, namespace `shop`.\n\n| Fact | Value |\n|---|---|\n| object | Deployment `cart` |\n| reason | `OOMKilled` |\n| fix (not applied) | raise the limit |\n"

func TestParseExpectedReadsAlertAndFacts(t *testing.T) {
	expected, err := ParseExpected("oom", oomExpected)
	if err != nil {
		t.Fatal(err)
	}
	if expected.Alert != "LabPodOOMKilled" || expected.Scenario != "oom" {
		t.Fatalf("alert %q scenario %q", expected.Alert, expected.Scenario)
	}
	if len(expected.Facts) != 3 || expected.Facts[0] != (Fact{Name: "object", Value: "Deployment `cart`"}) {
		t.Fatalf("facts %+v", expected.Facts)
	}
	if cause := expected.CauseFacts(); len(cause) != 2 || cause[1].Name != "reason" {
		t.Fatalf("cause facts %+v", cause)
	}
	if !expected.Facts[2].IsFix() || expected.Facts[0].IsFix() {
		t.Fatal("the fix row is the one starting with fix")
	}
	if expected.Facts[1].Line() != "reason: `OOMKilled`" {
		t.Fatal(expected.Facts[1].Line())
	}
}

func TestParseExpectedRefusesWhatIsNotAScenario(t *testing.T) {
	if _, err := ParseExpected("x", "no alert here\n| Fact | Value |\n|---|---|\n| a | b |\n"); err == nil || !strings.Contains(err.Error(), "no alert") {
		t.Fatalf("no alert: %v", err)
	}
	if _, err := ParseExpected("x", "Alert: `A`\n| Fact | Value |\n|---|---|\n| fix (not applied) | b |\n"); err == nil || !strings.Contains(err.Error(), "no fact table") {
		t.Fatalf("only a fix: %v", err)
	}
	if _, err := ParseExpected("x", "Alert: `A`\n| a | b | c |\n"); err == nil {
		t.Fatal("a three-column row is not a fact")
	}
}
