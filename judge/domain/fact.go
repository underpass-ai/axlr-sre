// Package domain is the judge's model: the facts a scenario expects, the
// verdict Jev gives on each, the score of a diagnosis and the result of a
// scenario's run. It knows no HTTP, no shell and no file.
package domain

import "strings"

// Fact is one row of a scenario's expected.md: what a correct diagnosis
// must say, named (object, reason, limit…) and valued in words.
type Fact struct {
	Name  string
	Value string
}

// IsFix reports whether the fact is the fix the bots must not apply; it is
// shown but never counted toward the cause.
func (f Fact) IsFix() bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimSpace(f.Name)), "fix")
}

// Line renders the fact as "name: value".
func (f Fact) Line() string { return f.Name + ": " + f.Value }
