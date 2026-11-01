package shell

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func lab(t *testing.T) Lab {
	t.Helper()
	dir := t.TempDir()
	write := func(path, content string) {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	write(filepath.Join(dir, "oom", "inject.sh"), "#!/bin/sh\necho injected > \"$(dirname \"$0\")/mark\"\n")
	write(filepath.Join(dir, "oom", "clear.sh"), "#!/bin/sh\nrm \"$(dirname \"$0\")/mark\"\n")
	write(filepath.Join(dir, "oom", "expected.md"), "Alert: `LabPodOOMKilled`\n\n| Fact | Value |\n|---|---|\n| object | cart |\n")
	write(filepath.Join(dir, "broken", "inject.sh"), "#!/bin/sh\nexit 3\n")
	write(filepath.Join(dir, "broken", "expected.md"), "no alert\n")
	write(filepath.Join(dir, "lib.sh"), "")
	write(filepath.Join(dir, "notes", "README.md"), "")
	return Lab{Dir: dir, Context: "kind-test", Settle: []string{"true"}, Stdout: os.Stdout}
}

func TestLabListsInjectsAndClears(t *testing.T) {
	l := lab(t)
	names, err := l.Scenarios()
	if err != nil || strings.Join(names, ",") != "broken,oom" {
		t.Fatalf("%v %v", names, err)
	}
	expected, err := l.Expected("oom")
	if err != nil || expected.Alert != "LabPodOOMKilled" || len(expected.Facts) != 1 {
		t.Fatalf("%+v %v", expected, err)
	}
	if _, err := l.Expected("broken"); err == nil {
		t.Fatal("no alert")
	}
	if _, err := l.Expected("nope"); err == nil {
		t.Fatal("no file")
	}
	ctx := context.Background()
	if err := l.Inject(ctx, "oom"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(l.Dir, "oom", "mark")); err != nil {
		t.Fatal("inject.sh ran")
	}
	if err := l.Clear(ctx, "oom"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(l.Dir, "oom", "mark")); err == nil {
		t.Fatal("clear.sh ran")
	}
	if err := l.Inject(ctx, "broken"); err == nil || !strings.Contains(err.Error(), "exited 3") {
		t.Fatalf("%v", err)
	}
	if err := l.Clear(ctx, "broken"); err == nil {
		t.Fatal("no clear.sh")
	}
	l.Settle = []string{"false"}
	if err := l.Clear(ctx, "oom"); err == nil {
		t.Fatal("a settle that fails")
	}
	l.Settle = nil
	l.Context = "kind-nowhere"
	if err := l.Clear(ctx, "oom"); err == nil {
		t.Fatal("kubectl against no cluster fails or is missing")
	}
	if _, err := (Lab{Dir: "/nowhere"}).Scenarios(); err == nil {
		t.Fatal("no directory")
	}
}
