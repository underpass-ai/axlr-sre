// Package results keeps one JSON file per scenario, so a run that breaks
// resumes from what landed and the table is rebuilt from the files.
package results

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"

	"github.com/underpass-ai/axlr-sre/judge/domain"
)

// Store is a directory of <scenario>.json.
type Store struct {
	Dir string
}

// Keep writes the scenario's result.
func (s Store) Keep(result domain.Result) error {
	if err := os.MkdirAll(s.Dir, 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(s.Dir, result.Scenario+".json"), data, 0o644)
}

// Load reads every result in the directory.
func (s Store) Load() (domain.Run, error) {
	entries, err := os.ReadDir(s.Dir)
	if err != nil {
		if os.IsNotExist(err) {
			return domain.NewRun(nil), nil
		}
		return domain.Run{}, err
	}
	var results []domain.Result
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(s.Dir, e.Name()))
		if err != nil {
			return domain.Run{}, err
		}
		var result domain.Result
		if err := json.Unmarshal(data, &result); err != nil {
			return domain.Run{}, err
		}
		results = append(results, result)
	}
	return domain.NewRun(results), nil
}
